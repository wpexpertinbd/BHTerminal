//! SFTP: pure wire-format checks that run anywhere, plus a full round trip
//! against a REAL sftp-server. The second kind is what actually proves the
//! client, because a protocol you only test against your own encoder agrees
//! with itself by construction.

use bhterm_core::sftp::{self, Attributes, Reader, SftpClient, MAX_PACKET};

// ── wire format (no process, runs on every platform) ──────────────────────

#[test]
fn rejects_a_length_field_that_would_have_us_allocate_the_world() {
    // The length prefix is the first thing a hostile server controls.
    let mut r = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let err = r.bytes().unwrap_err();
    assert!(format!("{err}").contains("exceeds the limit"), "got: {err}");
    assert!(MAX_PACKET <= 4 * 1024 * 1024);
}

#[test]
fn a_truncated_packet_is_an_error_not_a_panic() {
    let mut r = Reader::new(&[0, 0, 0, 10, b'a', b'b']); // claims 10, has 2
    assert!(r.bytes().is_err());
    let mut r = Reader::new(&[0, 0]);
    assert!(r.u32().is_err());
}

#[test]
fn parses_attributes_and_classifies_file_types() {
    // flags = SIZE | PERMISSIONS, size 1234, mode 0o040755 (a directory)
    let mut buf = vec![];
    buf.extend_from_slice(&(0x1u32 | 0x4).to_be_bytes());
    buf.extend_from_slice(&1234u64.to_be_bytes());
    buf.extend_from_slice(&0o040755u32.to_be_bytes());
    let mut r = Reader::new(&buf);
    let a = Attributes::parse(&mut r).unwrap();
    assert_eq!(a.size, Some(1234));
    assert!(a.is_dir());
    assert!(!a.is_symlink());

    // A regular file must not be mistaken for one.
    let mut buf = vec![];
    buf.extend_from_slice(&0x4u32.to_be_bytes());
    buf.extend_from_slice(&0o100644u32.to_be_bytes());
    let a = Attributes::parse(&mut Reader::new(&buf)).unwrap();
    assert!(!a.is_dir());
}

#[test]
fn remote_paths_are_always_slash_separated() {
    // std::path would emit backslashes on Windows and break every request.
    assert_eq!(sftp::join("/var/www", "index.php"), "/var/www/index.php");
    assert_eq!(sftp::join("/", "etc"), "/etc");
    assert_eq!(sftp::join("/a/", "b"), "/a/b");
    assert_eq!(sftp::parent_of("/var/www/html"), "/var/www");
    assert_eq!(sftp::parent_of("/var"), "/");
    assert_eq!(sftp::parent_of("/"), "/");
}

// ── full round trip against a real server ─────────────────────────────────

#[cfg(unix)]
fn sftp_server_path() -> Option<&'static str> {
    ["/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server", "/usr/lib/ssh/sftp-server"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
}

#[cfg(unix)]
#[test]
fn round_trips_against_a_real_sftp_server() {
    use std::io::Cursor;
    use std::process::{Command, Stdio};

    let Some(server) = sftp_server_path() else {
        eprintln!("no sftp-server on this machine — skipping");
        return;
    };

    let mut child = Command::new(server)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sftp-server");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut sftp = SftpClient::connect(stdout, stdin).expect("SFTP handshake");

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_str().unwrap().to_string();

    // realpath on a known directory
    let resolved = sftp.realpath(&base).expect("realpath");
    assert!(resolved.ends_with(dir.path().file_name().unwrap().to_str().unwrap()));

    // mkdir
    let sub = sftp::join(&base, "sub");
    sftp.mkdir(&sub).expect("mkdir");
    assert!(sftp.stat(&sub).expect("stat dir").is_dir());

    // upload → the bytes must land on disk exactly
    let remote = sftp::join(&sub, "hello.txt");
    let payload = b"hello from bhterm-core\n".repeat(500); // ~11 KB, multi-chunk
    let written = sftp
        .upload(&remote, &mut Cursor::new(payload.clone()))
        .expect("upload");
    assert_eq!(written as usize, payload.len());
    assert_eq!(std::fs::read(&remote).unwrap(), payload, "uploaded bytes must match");

    // stat reports the size the server actually has
    assert_eq!(sftp.stat(&remote).unwrap().size, Some(payload.len() as u64));

    // list_dir sees it, and hides . / ..
    let entries = sftp.list_dir(&sub).expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "hello.txt");
    assert!(!entries.iter().any(|e| e.filename == "." || e.filename == ".."));

    // download streams it back byte-for-byte
    let mut got = Vec::new();
    let n = sftp.download(&remote, &mut got).expect("download");
    assert_eq!(n as usize, payload.len());
    assert_eq!(got, payload, "downloaded bytes must match");

    // rename
    let renamed = sftp::join(&sub, "renamed.txt");
    sftp.rename(&remote, &renamed).expect("rename");
    assert!(sftp.stat(&remote).is_err());
    assert!(sftp.stat(&renamed).is_ok());

    // remove + rmdir
    sftp.remove(&renamed).expect("remove");
    sftp.rmdir(&sub).expect("rmdir");
    assert!(sftp.stat(&sub).is_err());

    let _ = child.kill();
}

#[cfg(unix)]
#[test]
fn a_missing_file_is_an_ordinary_outcome_not_a_crash() {
    use std::process::{Command, Stdio};
    let Some(server) = sftp_server_path() else { return };
    let mut child = Command::new(server)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    let mut sftp =
        SftpClient::connect(child.stdout.take().unwrap(), child.stdin.take().unwrap()).unwrap();

    let err = sftp.stat("/definitely/not/here/at/all").unwrap_err();
    assert!(err.is_no_such_file(), "got: {err}");
    // …and the session is still usable afterwards.
    assert!(sftp.realpath("/").is_ok(), "one failed request must not poison the stream");
    let _ = child.kill();
}
