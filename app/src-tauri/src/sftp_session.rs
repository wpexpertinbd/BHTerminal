//! One live SFTP session per host.
//!
//! Per host, not per tab, and kept alive: the Swift build originally tore the
//! channel down and rebuilt it on every tab switch, so the file pane spent its
//! time reconnecting and forgot which directory you were in.
//!
//! The channel runs over `ssh -s … sftp`, which reuses the terminal's
//! authenticated connection where the platform supports multiplexing. On
//! Windows OpenSSH has no ControlMaster, so this authenticates separately —
//! the stored secret makes that invisible, but it IS a second connection.

use bhterm_core::sftp::{self, DirEntry, SftpClient, SftpError};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Cursor;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub mtime: Option<u32>,
}

impl From<&DirEntry> for Entry {
    fn from(e: &DirEntry) -> Self {
        Entry {
            name: e.filename.clone(),
            is_dir: e.attributes.is_dir(),
            is_symlink: e.attributes.is_symlink(),
            size: e.attributes.size,
            permissions: e.attributes.permissions,
            mtime: e.attributes.mtime,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    /// The directory the session logged into — what a window title's "~"
    /// expands against, since only the remote knows its own home.
    pub home: String,
    pub entries: Vec<Entry>,
}

pub struct SftpSession {
    client: Mutex<SftpClient<ChildStdout, ChildStdin>>,
    child: Mutex<Child>,
    home: String,
    /// How this session was opened, so a dead one can be rebuilt without
    /// asking the UI to start again.
    program: String,
    args: Vec<String>,
}

impl SftpSession {
    /// Whether the ssh behind this session is still running. A child that has
    /// exited leaves every later call failing on a closed pipe — on Windows
    /// literally "The pipe is being closed. (os error 232)".
    fn is_alive(&self) -> bool {
        match self.child.lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }

    fn list(&self, path: &str) -> Result<Listing, String> {
        let mut client = self.client.lock().unwrap();
        let resolved = client.realpath(path).map_err(stringify)?;
        let mut entries: Vec<Entry> =
            client.list_dir(&resolved).map_err(stringify)?.iter().map(Entry::from).collect();
        // Directories first, then case-insensitive by name — the order people
        // expect from a file browser.
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(Listing { path: resolved, home: self.home.clone(), entries })
    }
}

fn stringify(e: SftpError) -> String {
    e.to_string()
}

#[derive(Default)]
pub struct SftpRegistry {
    sessions: Mutex<HashMap<String, Arc<SftpSession>>>,
}

impl SftpRegistry {
    pub fn existing(&self, host_key: &str) -> Option<Arc<SftpSession>> {
        self.sessions.lock().unwrap().get(host_key).cloned()
    }

    /// Opens the channel, retrying while the terminal's shared connection
    /// comes up. BatchMode ssh has no tty, so until the master exists it
    /// fails immediately rather than prompting — retrying is what covers the
    /// user typing a passphrase in the terminal next door.
    pub fn connect(
        &self,
        host_key: String,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<Arc<SftpSession>, String> {
        if let Some(existing) = self.existing(&host_key) {
            return Ok(existing);
        }

        let deadline = Instant::now() + timeout;
        let last_error: String;
        loop {
            match Self::try_open(program, args) {
                Ok(session) => {
                    let session = Arc::new(session);
                    self.sessions.lock().unwrap().insert(host_key, session.clone());
                    return Ok(session);
                }
                Err(e) => {
                    if Instant::now() >= deadline {
                        last_error = e;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1200));
        }
        Err(last_error)
    }

    /// How long ONE attempt may spend in the SFTP handshake before the child
    /// is killed. Without this a wedged ssh — one waiting on a prompt it has
    /// nowhere to show, say — blocks the read forever, and since the child
    /// now has no console window on Windows it would be an INVISIBLE orphan.
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(12);

    fn try_open(program: &str, args: &[String]) -> Result<SftpSession, String> {
        let mut child = crate::proc::command(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start {program}: {e}"))?;

        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        // The handshake reads from the child, which can block indefinitely,
        // so it runs on its own thread and the child is killed if it does.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // The receiver is gone on timeout; the send failing is expected.
            let _ = tx.send(SftpClient::connect(stdout, stdin));
        });

        let mut client = match rx.recv_timeout(Self::HANDSHAKE_TIMEOUT) {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
            Err(_) => {
                // Killing the child also unblocks the thread's read.
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "the SFTP channel did not answer — the server may be asking for something ssh \
                     cannot ask here (an unknown host key, or a password with no key or agent). \
                     Open a terminal to that host first."
                        .into(),
                );
            }
        };
        // Where the session landed IS the remote home.
        let home = client.realpath(".").unwrap_or_else(|_| "/".to_string());
        Ok(SftpSession {
            client: Mutex::new(client),
            child: Mutex::new(child),
            home,
            program: program.to_string(),
            args: args.to_vec(),
        })
    }

    pub fn list(&self, host_key: &str, path: &str) -> Result<Listing, String> {
        self.session(host_key)?.list(path)
    }

    pub fn mkdir(&self, host_key: &str, path: &str) -> Result<(), String> {
        let s = self.session(host_key)?;
        let mut c = s.client.lock().unwrap();
        c.mkdir(path).map_err(stringify)
    }

    /// Creates an empty file.
    ///
    /// ⚠️ An existing path is REFUSED rather than truncated: the SFTP open
    /// this is built on would happily empty a file that is already there,
    /// and "New file…" must never be a way to wipe one.
    pub fn create_file(&self, host_key: &str, path: &str) -> Result<(), String> {
        let s = self.session(host_key)?;
        let mut c = s.client.lock().unwrap();
        match c.lstat(path) {
            Ok(_) => return Err("something with that name is already here".into()),
            Err(e) if e.is_no_such_file() => {}
            Err(e) => return Err(stringify(e)),
        }
        c.upload(path, &mut Cursor::new(Vec::new())).map_err(stringify)?;
        Ok(())
    }

    pub fn rename(&self, host_key: &str, from: &str, to: &str) -> Result<(), String> {
        let s = self.session(host_key)?;
        let mut c = s.client.lock().unwrap();
        c.rename(from, to).map_err(stringify)
    }

    pub fn delete(&self, host_key: &str, path: &str, is_dir: bool) -> Result<(), String> {
        let s = self.session(host_key)?;
        let mut c = s.client.lock().unwrap();
        if is_dir { c.rmdir(path).map_err(stringify) } else { c.remove(path).map_err(stringify) }
    }

    /// Streams to disk rather than buffering: a huge file must not be held in
    /// memory just to be saved.
    pub fn download(&self, host_key: &str, remote: &str, local: &std::path::Path) -> Result<u64, String> {
        let s = self.session(host_key)?;
        let mut file = std::fs::File::create(local).map_err(|e| e.to_string())?;
        let mut c = s.client.lock().unwrap();
        c.download(remote, &mut file).map_err(stringify)
    }

    pub fn upload(&self, host_key: &str, local: &std::path::Path, remote: &str) -> Result<u64, String> {
        let s = self.session(host_key)?;
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        let mut c = s.client.lock().unwrap();
        c.upload(remote, &mut Cursor::new(bytes)).map_err(stringify)
    }

    pub fn disconnect(&self, host_key: &str) {
        if let Some(session) = self.sessions.lock().unwrap().remove(host_key) {
            if let Ok(mut child) = session.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// SFTP rides the terminal's connection, so a host with no terminal left
    /// open has no business holding one.
    pub fn prune(&self, keep: &[String]) {
        let stale: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .keys()
            .filter(|k| !keep.contains(k))
            .cloned()
            .collect();
        for key in stale {
            self.disconnect(&key);
        }
    }

    pub fn disconnect_all(&self) {
        let keys: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for k in keys {
            self.disconnect(&k);
        }
    }

    /// The live session for a host, reopening it if the old one died.
    ///
    /// An ssh can go away for reasons that are nobody's mistake — the network
    /// drops, the server reboots, or (on Windows, before the console window
    /// was suppressed) somebody closes the black window it was running in.
    /// Leaving the dead session in the map made every later click fail with a
    /// closed-pipe error until the app was restarted; rebuilding it quietly
    /// is what the user means by "it should just work".
    fn session(&self, host_key: &str) -> Result<Arc<SftpSession>, String> {
        let existing = self
            .existing(host_key)
            .ok_or_else(|| "no SFTP session for this host".to_string())?;
        if existing.is_alive() {
            return Ok(existing);
        }

        let (program, args) = (existing.program.clone(), existing.args.clone());
        self.sessions.lock().unwrap().remove(host_key);
        // One quick attempt: this is a click waiting on it, not a login.
        let session = Arc::new(Self::try_open(&program, &args)?);
        self.sessions
            .lock()
            .unwrap()
            .insert(host_key.to_string(), session.clone());
        Ok(session)
    }
}

pub fn join(base: &str, name: &str) -> String {
    sftp::join(base, name)
}

pub type SharedSftp = Arc<SftpRegistry>;
