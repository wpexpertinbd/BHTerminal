//! A minimal SFTP v3 client.
//!
//! It speaks the wire protocol directly over a transport the caller supplies
//! — in the app that is the stdin/stdout of `ssh -s … sftp`, which rides the
//! terminal's already-authenticated connection, so keys, agent, passphrases,
//! 2FA and ProxyJump all work with no second sign-in and no SSH stack of our
//! own.
//!
//! Everything a hostile or broken server could use to hurt us is bounded:
//! packet length, directory length, and in-memory file size. A server is not
//! trusted just because the user chose to connect to it.

use std::collections::HashMap;
use std::io::{Read, Write};
use thiserror::Error;

// ── protocol constants (RFC draft-ietf-secsh-filexfer-02) ──────────────────
const FXP_INIT: u8 = 1;
const FXP_VERSION: u8 = 2;
const FXP_OPEN: u8 = 3;
const FXP_CLOSE: u8 = 4;
const FXP_READ: u8 = 5;
const FXP_WRITE: u8 = 6;
const FXP_LSTAT: u8 = 7;
const FXP_OPENDIR: u8 = 11;
const FXP_READDIR: u8 = 12;
const FXP_REMOVE: u8 = 13;
const FXP_MKDIR: u8 = 14;
const FXP_RMDIR: u8 = 15;
const FXP_REALPATH: u8 = 16;
const FXP_STAT: u8 = 17;
const FXP_RENAME: u8 = 18;

const FXP_STATUS: u8 = 101;
const FXP_HANDLE: u8 = 102;
const FXP_DATA: u8 = 103;
const FXP_NAME: u8 = 104;
const FXP_ATTRS: u8 = 105;

const SSH_FX_OK: u32 = 0;
const SSH_FX_EOF: u32 = 1;

const ATTR_SIZE: u32 = 0x1;
const ATTR_UIDGID: u32 = 0x2;
const ATTR_PERMISSIONS: u32 = 0x4;
const ATTR_ACMODTIME: u32 = 0x8;
const ATTR_EXTENDED: u32 = 0x8000_0000;

const FXF_READ: u32 = 0x1;
const FXF_WRITE: u32 = 0x2;
const FXF_CREAT: u32 = 0x8;
const FXF_TRUNC: u32 = 0x10;

pub const VERSION: u32 = 3;

/// 4 MB. A length field is the first thing a hostile server controls, so it
/// is checked before a single byte is allocated.
pub const MAX_PACKET: usize = 4 * 1024 * 1024;
/// Stops a server (or a pathological directory) from growing the listing
/// without limit.
pub const MAX_DIR_ENTRIES: usize = 2_000_000;
/// Reads are chunked; this is the per-request ask, well under MAX_PACKET.
const READ_CHUNK: u32 = 32 * 1024;

#[derive(Debug, Error)]
pub enum SftpError {
    #[error("SFTP I/O error: {0}")]
    Io(String),
    #[error("the server broke the SFTP protocol: {0}")]
    Protocol(String),
    #[error("SFTP error {code}: {message}")]
    Status { code: u32, message: String },
    #[error("the server offered SFTP v{0}; this client speaks v3")]
    UnsupportedVersion(u32),
}

impl SftpError {
    /// "No such file" and friends are ordinary outcomes, not faults.
    pub fn is_no_such_file(&self) -> bool {
        matches!(self, SftpError::Status { code: 2, .. })
    }
}

type Result<T> = std::result::Result<T, SftpError>;

// ── wire encoding ──────────────────────────────────────────────────────────

#[derive(Default)]
struct Buf(Vec<u8>);

impl Buf {
    fn u32(&mut self, v: u32) -> &mut Self { self.0.extend_from_slice(&v.to_be_bytes()); self }
    fn u64(&mut self, v: u64) -> &mut Self { self.0.extend_from_slice(&v.to_be_bytes()); self }
    fn string(&mut self, s: &str) -> &mut Self { self.bytes(s.as_bytes()) }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        self
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    pub fn is_at_end(&self) -> bool {
        self.pos >= self.data.len()
    }
    fn need(&self, n: usize) -> Result<()> {
        if self.pos + n > self.data.len() {
            return Err(SftpError::Protocol("packet ended mid-field".into()));
        }
        Ok(())
    }
    pub fn u32(&mut self) -> Result<u32> {
        self.need(4)?;
        let v = u32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }
    pub fn u64(&mut self) -> Result<u64> {
        self.need(8)?;
        let v = u64::from_be_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        // A length is server-controlled: bound it before trusting it.
        if len > MAX_PACKET {
            return Err(SftpError::Protocol(format!("field of {len} bytes exceeds the limit")));
        }
        self.need(len)?;
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }
    pub fn string(&mut self) -> Result<String> {
        // Remote filenames are not guaranteed UTF-8; a lossy name is better
        // than refusing to list the directory it sits in.
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
}

/// File metadata, as much of it as the server chose to send.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes {
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub mtime: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

impl Attributes {
    pub fn is_dir(&self) -> bool {
        self.permissions.is_some_and(|p| p & 0o170000 == 0o040000)
    }
    pub fn is_symlink(&self) -> bool {
        self.permissions.is_some_and(|p| p & 0o170000 == 0o120000)
    }

    pub fn parse(r: &mut Reader) -> Result<Attributes> {
        let flags = r.u32()?;
        let mut a = Attributes::default();
        if flags & ATTR_SIZE != 0 { a.size = Some(r.u64()?); }
        if flags & ATTR_UIDGID != 0 { a.uid = Some(r.u32()?); a.gid = Some(r.u32()?); }
        if flags & ATTR_PERMISSIONS != 0 { a.permissions = Some(r.u32()?); }
        if flags & ATTR_ACMODTIME != 0 { let _atime = r.u32()?; a.mtime = Some(r.u32()?); }
        if flags & ATTR_EXTENDED != 0 {
            let count = r.u32()?;
            // Bounded by the packet itself: each pair costs at least 8 bytes,
            // so a huge count simply runs out of data below.
            for _ in 0..count {
                if r.is_at_end() { break; }
                let _ = r.bytes()?;
                let _ = r.bytes()?;
            }
        }
        Ok(a)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub filename: String,
    pub longname: String,
    pub attributes: Attributes,
}

// ── the client ─────────────────────────────────────────────────────────────

pub struct SftpClient<R: Read, W: Write> {
    reader: R,
    writer: W,
    next_id: u32,
}

enum Response {
    Status { code: u32, message: String },
    Handle(Vec<u8>),
    Data(Vec<u8>),
    Name(Vec<DirEntry>),
    Attrs(Attributes),
}

impl<R: Read, W: Write> SftpClient<R, W> {
    /// Performs the version handshake. A server that answers with anything
    /// other than VERSION is not an SFTP server we can talk to.
    pub fn connect(reader: R, writer: W) -> Result<Self> {
        let mut client = SftpClient { reader, writer, next_id: 1 };
        let mut body = Buf::default();
        body.u32(VERSION);
        client.send(FXP_INIT, &body.0)?;

        let (kind, payload) = client.read_packet()?;
        if kind != FXP_VERSION {
            return Err(SftpError::Protocol(format!("expected VERSION, got packet type {kind}")));
        }
        let mut r = Reader::new(&payload);
        let version = r.u32()?;
        if version < VERSION {
            return Err(SftpError::UnsupportedVersion(version));
        }
        Ok(client)
    }

    fn send(&mut self, kind: u8, body: &[u8]) -> Result<()> {
        let len = (body.len() + 1) as u32;
        let mut out = Vec::with_capacity(body.len() + 5);
        out.extend_from_slice(&len.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(body);
        self.writer.write_all(&out).map_err(|e| SftpError::Io(e.to_string()))?;
        self.writer.flush().map_err(|e| SftpError::Io(e.to_string()))
    }

    fn read_packet(&mut self) -> Result<(u8, Vec<u8>)> {
        let mut len_buf = [0u8; 4];
        self.reader.read_exact(&mut len_buf).map_err(|e| SftpError::Io(e.to_string()))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        // ⚠️ Checked BEFORE allocating: this is the one field that would let
        // a hostile server ask us to reserve gigabytes.
        if len == 0 || len > MAX_PACKET {
            return Err(SftpError::Protocol(format!("packet length {len} is out of range")));
        }
        let mut body = vec![0u8; len];
        self.reader.read_exact(&mut body).map_err(|e| SftpError::Io(e.to_string()))?;
        let kind = body[0];
        Ok((kind, body.split_off(1)))
    }

    fn request(&mut self, kind: u8, body: &[u8]) -> Result<Response> {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        let mut framed = Buf::default();
        framed.u32(id);
        framed.0.extend_from_slice(body);
        self.send(kind, &framed.0)?;

        loop {
            let (rkind, payload) = self.read_packet()?;
            let mut r = Reader::new(&payload);
            let rid = r.u32()?;
            if rid != id {
                // Requests are issued one at a time, so a mismatched id means
                // the stream is out of step — keep reading rather than
                // silently pairing the wrong reply with this request.
                continue;
            }
            return match rkind {
                FXP_STATUS => {
                    let code = r.u32()?;
                    let message = r.string().unwrap_or_default();
                    Ok(Response::Status { code, message })
                }
                FXP_HANDLE => Ok(Response::Handle(r.bytes()?.to_vec())),
                FXP_DATA => Ok(Response::Data(r.bytes()?.to_vec())),
                FXP_ATTRS => Ok(Response::Attrs(Attributes::parse(&mut r)?)),
                FXP_NAME => {
                    let count = r.u32()? as usize;
                    if count > MAX_DIR_ENTRIES {
                        return Err(SftpError::Protocol(format!(
                            "directory listing claims {count} entries"
                        )));
                    }
                    let mut entries = Vec::new();
                    for _ in 0..count {
                        // Trust the DATA, not the count: stop at the end of
                        // the packet however many were promised.
                        if r.is_at_end() { break; }
                        let filename = r.string()?;
                        let longname = r.string()?;
                        let attributes = Attributes::parse(&mut r)?;
                        entries.push(DirEntry { filename, longname, attributes });
                    }
                    Ok(Response::Name(entries))
                }
                other => Err(SftpError::Protocol(format!("unexpected packet type {other}"))),
            };
        }
    }

    fn expect_ok(&mut self, kind: u8, body: &[u8]) -> Result<()> {
        match self.request(kind, body)? {
            Response::Status { code: SSH_FX_OK, .. } => Ok(()),
            Response::Status { code, message } => Err(SftpError::Status { code, message }),
            _ => Err(SftpError::Protocol("expected a status reply".into())),
        }
    }

    pub fn realpath(&mut self, path: &str) -> Result<String> {
        let mut b = Buf::default();
        b.string(path);
        match self.request(FXP_REALPATH, &b.0)? {
            Response::Name(entries) => entries
                .first()
                .map(|e| e.filename.clone())
                .ok_or_else(|| SftpError::Protocol("REALPATH returned no name".into())),
            Response::Status { code, message } => Err(SftpError::Status { code, message }),
            _ => Err(SftpError::Protocol("expected a name reply".into())),
        }
    }

    pub fn stat(&mut self, path: &str) -> Result<Attributes> {
        self.stat_with(FXP_STAT, path)
    }

    pub fn lstat(&mut self, path: &str) -> Result<Attributes> {
        self.stat_with(FXP_LSTAT, path)
    }

    fn stat_with(&mut self, kind: u8, path: &str) -> Result<Attributes> {
        let mut b = Buf::default();
        b.string(path);
        match self.request(kind, &b.0)? {
            Response::Attrs(a) => Ok(a),
            Response::Status { code, message } => Err(SftpError::Status { code, message }),
            _ => Err(SftpError::Protocol("expected an attrs reply".into())),
        }
    }

    pub fn list_dir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let mut b = Buf::default();
        b.string(path);
        let handle = match self.request(FXP_OPENDIR, &b.0)? {
            Response::Handle(h) => h,
            Response::Status { code, message } => return Err(SftpError::Status { code, message }),
            _ => return Err(SftpError::Protocol("expected a handle".into())),
        };

        let mut all = Vec::new();
        loop {
            let mut rb = Buf::default();
            rb.bytes(&handle);
            match self.request(FXP_READDIR, &rb.0)? {
                Response::Name(mut entries) => {
                    if entries.is_empty() { break; }
                    all.append(&mut entries);
                    if all.len() > MAX_DIR_ENTRIES {
                        self.close_handle(&handle);
                        return Err(SftpError::Protocol("directory listing too large".into()));
                    }
                }
                Response::Status { code: SSH_FX_EOF, .. } => break,
                Response::Status { code, message } => {
                    self.close_handle(&handle);
                    return Err(SftpError::Status { code, message });
                }
                _ => break,
            }
        }
        self.close_handle(&handle);
        all.retain(|e| e.filename != "." && e.filename != "..");
        Ok(all)
    }

    fn close_handle(&mut self, handle: &[u8]) {
        let mut b = Buf::default();
        b.bytes(handle);
        let _ = self.expect_ok(FXP_CLOSE, &b.0);
    }

    /// Streams a remote file into `sink` rather than buffering it, so a huge
    /// (or endless) file can't exhaust memory.
    pub fn download(&mut self, path: &str, sink: &mut impl Write) -> Result<u64> {
        let mut b = Buf::default();
        b.string(path).u32(FXF_READ).u32(0);
        let handle = match self.request(FXP_OPEN, &b.0)? {
            Response::Handle(h) => h,
            Response::Status { code, message } => return Err(SftpError::Status { code, message }),
            _ => return Err(SftpError::Protocol("expected a handle".into())),
        };

        let mut offset: u64 = 0;
        loop {
            let mut rb = Buf::default();
            rb.bytes(&handle).u64(offset).u32(READ_CHUNK);
            match self.request(FXP_READ, &rb.0)? {
                Response::Data(chunk) => {
                    if chunk.is_empty() { break; }
                    sink.write_all(&chunk).map_err(|e| SftpError::Io(e.to_string()))?;
                    offset += chunk.len() as u64;
                }
                Response::Status { code: SSH_FX_EOF, .. } => break,
                Response::Status { code, message } => {
                    self.close_handle(&handle);
                    return Err(SftpError::Status { code, message });
                }
                _ => break,
            }
        }
        self.close_handle(&handle);
        Ok(offset)
    }

    pub fn upload(&mut self, path: &str, source: &mut impl Read) -> Result<u64> {
        let mut b = Buf::default();
        b.string(path).u32(FXF_WRITE | FXF_CREAT | FXF_TRUNC).u32(0);
        let handle = match self.request(FXP_OPEN, &b.0)? {
            Response::Handle(h) => h,
            Response::Status { code, message } => return Err(SftpError::Status { code, message }),
            _ => return Err(SftpError::Protocol("expected a handle".into())),
        };

        let mut offset: u64 = 0;
        let mut chunk = vec![0u8; READ_CHUNK as usize];
        loop {
            let n = source.read(&mut chunk).map_err(|e| SftpError::Io(e.to_string()))?;
            if n == 0 { break; }
            let mut wb = Buf::default();
            wb.bytes(&handle).u64(offset).bytes(&chunk[..n]);
            if let Err(e) = self.expect_ok(FXP_WRITE, &wb.0) {
                self.close_handle(&handle);
                return Err(e);
            }
            offset += n as u64;
        }
        self.close_handle(&handle);
        Ok(offset)
    }

    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        let mut b = Buf::default();
        b.string(path).u32(0); // no attribute flags
        self.expect_ok(FXP_MKDIR, &b.0)
    }

    pub fn rmdir(&mut self, path: &str) -> Result<()> {
        let mut b = Buf::default();
        b.string(path);
        self.expect_ok(FXP_RMDIR, &b.0)
    }

    pub fn remove(&mut self, path: &str) -> Result<()> {
        let mut b = Buf::default();
        b.string(path);
        self.expect_ok(FXP_REMOVE, &b.0)
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let mut b = Buf::default();
        b.string(from).string(to);
        self.expect_ok(FXP_RENAME, &b.0)
    }
}

/// Joins a remote path. Remote paths are always `/`-separated, whatever the
/// LOCAL platform uses — std::path would produce backslashes on Windows and
/// quietly break every request.
pub fn join(base: &str, name: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub fn parent_of(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

/// Convenience for callers holding a map of extensions → icon, etc.
pub type Listing = HashMap<String, DirEntry>;
