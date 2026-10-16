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
//!
//! ⚠️ Where there IS a shared connection, a host that needs a PASSWORD must
//! never be dialled by the file pane itself: the channel runs without a tty
//! (BatchMode), so every such attempt is a guaranteed failed login. Retried
//! once every 1.2 s while someone typed a password next door, that was ~35
//! failed logins in 45 s — enough to trip fail2ban ("Connection closed by …")
//! — and after the 45 s the pane gave up for good. So the pane WAITS for the
//! terminal's connection instead (`wait_for_master`), asking the local control
//! socket, which costs the server nothing.

use bhterm_core::sftp::{self, DirEntry, SftpClient, SftpError};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Environment the ssh was started with — on Windows the askpass hand-over
    /// of a saved password. Kept so a dropped connection can be rebuilt; it
    /// lives only as long as the session, in the same process memory that
    /// already holds the whole vault.
    env: Vec<(String, String)>,
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

/// `base`, plus the last thing ssh said, when it said something useful.
///
/// Control sequences and the harmless "Permanently added … to known hosts"
/// notice are dropped, and the result is capped — this lands in a status line.
fn with_ssh_said(base: String, said: &Mutex<String>) -> String {
    let raw = said.lock().map(|g| g.clone()).unwrap_or_default();
    let clean = bhterm_core::prompt::strip_ansi(&raw).replace('\r', "");
    let lines: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))
        .collect();
    if lines.is_empty() {
        return base;
    }
    let tail = lines[lines.len().saturating_sub(2)..].join(" ");
    let short: String = tail.chars().take(240).collect();
    format!("{base} — ssh said: {short}")
}

/// Hands every wait its own number, so a newer wait for the same host (or an
/// explicit cancel) can tell an older one to stop.
static WAIT_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct SftpRegistry {
    sessions: Mutex<HashMap<String, Arc<SftpSession>>>,
    /// Hosts whose file pane is waiting for the terminal's connection, and
    /// which wait is the current one. Removing an entry cancels the wait.
    waiting: Mutex<HashMap<String, u64>>,
}

impl SftpRegistry {
    pub fn existing(&self, host_key: &str) -> Option<Arc<SftpSession>> {
        self.sessions.lock().unwrap().get(host_key).cloned()
    }

    /// Opens the channel.
    ///
    /// With a `timeout` it keeps trying every 1.2 s until then — for the case
    /// where the login is a key (no prompt, so a failure is a real failure and
    /// retrying is cheap). A ZERO timeout means exactly ONE attempt: used
    /// where each try is a login the server counts, and where the caller has a
    /// better plan than retrying (`wait_for_master`).
    pub fn connect(
        &self,
        host_key: String,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<Arc<SftpSession>, String> {
        if let Some(existing) = self.existing(&host_key) {
            return Ok(existing);
        }

        let deadline = Instant::now() + timeout;
        let last_error: String;
        loop {
            match Self::try_open(program, args, env) {
                Ok(session) => {
                    let session = Arc::new(session);
                    let mut sessions = self.sessions.lock().unwrap();
                    // Two connects can race (a double click on "Try again", a
                    // remount). Keep the first session and stop the second's
                    // ssh — replacing the map entry would leave it running
                    // with nothing able to close it.
                    if let Some(winner) = sessions.get(&host_key) {
                        let winner = winner.clone();
                        drop(sessions);
                        if let Ok(mut child) = session.child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        return Ok(winner);
                    }
                    sessions.insert(host_key, session.clone());
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

    /// Blocks until the terminal's shared connection to this host exists.
    ///
    /// `check_args` is `ssh -O check`, which talks only to the local control
    /// socket: no network, no login, nothing for the server to log — so it is
    /// polled every second for as long as it takes someone to type or paste a
    /// password, with no cost to the server and no deadline that gives up on
    /// them after 45 seconds. Superseded by a newer wait for the same host, or
    /// cancelled when the host's terminal goes away (`prune`/`disconnect`).
    pub fn wait_for_master(
        &self,
        host_key: &str,
        program: &str,
        check_args: &[String],
        max_wait: Duration,
    ) -> Result<(), String> {
        let mine = WAIT_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        self.waiting.lock().unwrap().insert(host_key.to_string(), mine);
        let deadline = Instant::now() + max_wait;
        let done = |registry: &Self| {
            let mut waiting = registry.waiting.lock().unwrap();
            if waiting.get(host_key) == Some(&mine) {
                waiting.remove(host_key);
            }
        };
        loop {
            if self.waiting.lock().unwrap().get(host_key) != Some(&mine) {
                return Err("stopped waiting for the terminal login".into());
            }
            if Self::master_is_up(program, check_args) {
                done(self);
                return Ok(());
            }
            if Instant::now() >= deadline {
                done(self);
                return Err(
                    "the terminal did not finish logging in. Log in there, then press ⟳ here."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// `ssh -O check`, with a deadline. A control socket whose master process
    /// is alive but wedged can leave the check hanging; without a limit that
    /// would pin the waiting thread, deaf to cancellation and to its own
    /// 15-minute deadline.
    fn master_is_up(program: &str, check_args: &[String]) -> bool {
        let Ok(mut child) = crate::proc::command(program)
            .args(check_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }

    /// Stops a pending wait (the file pane went away). A wait that has already
    /// finished is unaffected.
    pub fn cancel_wait(&self, host_key: &str) {
        self.waiting.lock().unwrap().remove(host_key);
    }

    /// How long ONE attempt may spend in the SFTP handshake before the child
    /// is killed. Without this a wedged ssh — one waiting on a prompt it has
    /// nowhere to show, say — blocks the read forever, and since the child
    /// now has no console window on Windows it would be an INVISIBLE orphan.
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(12);

    fn try_open(
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<SftpSession, String> {
        let mut child = crate::proc::command(program)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't start {program}: {e}"))?;

        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        // What ssh says on stderr is the only explanation of a failed login
        // ("Permission denied", "Connection closed by …"). It used to be
        // thrown away, which left "failed to fill whole buffer" as the whole
        // story. Drained for the life of the child so a chatty ssh can never
        // block on a full pipe.
        let said = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let said = Arc::clone(&said);
            std::thread::spawn(move || {
                let mut buf = [0u8; 512];
                while let Ok(n) = stderr.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut text = said.lock().unwrap();
                    text.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if text.len() > 2000 {
                        let mut cut = text.len() - 2000;
                        while !text.is_char_boundary(cut) {
                            cut += 1;
                        }
                        text.drain(..cut);
                    }
                }
            });
        }

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
                // The reader thread needs a moment to hand over what ssh said.
                std::thread::sleep(Duration::from_millis(150));
                return Err(with_ssh_said(e.to_string(), &said));
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
            env: env.to_vec(),
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
        self.waiting.lock().unwrap().remove(host_key);
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
        // A file pane waiting on a terminal that is gone has nothing to wait for.
        self.waiting.lock().unwrap().retain(|k, _| keep.contains(k));
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

        let (program, args, env) =
            (existing.program.clone(), existing.args.clone(), existing.env.clone());
        self.sessions.lock().unwrap().remove(host_key);
        // One quick attempt: this is a click waiting on it, not a login.
        let session = Arc::new(Self::try_open(&program, &args, &env)?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    /// A stand-in "ssh" that records every time it is started and then fails,
    /// like a password host dialled without a tty.
    fn counting_ssh(dir: &std::path::Path) -> Vec<String> {
        sh(&format!("echo x >> '{}'; echo 'Permission denied (publickey,password).' >&2; exit 255", dir.join("count").display()))
    }

    fn attempts(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("count")).map(|s| s.lines().count()).unwrap_or(0)
    }

    /// The hammering bug: a retry loop dialling a password host every 1.2 s
    /// for 45 s is ~35 failed logins. A zero timeout must be ONE attempt.
    #[test]
    fn a_zero_timeout_is_exactly_one_attempt() {
        let d = tempfile::tempdir().unwrap();
        let registry = SftpRegistry::default();
        let started = Instant::now();
        let result = registry.connect("h".into(), "/bin/sh", &counting_ssh(d.path()), &[], Duration::ZERO);
        assert!(result.is_err());
        assert_eq!(attempts(d.path()), 1, "must dial once, not retry");
        assert!(started.elapsed() < Duration::from_secs(5), "must not sleep-and-retry");
    }

    #[test]
    fn a_positive_timeout_retries_until_it_runs_out() {
        // The key-login behaviour is kept: a passphrase typed next door has time.
        let d = tempfile::tempdir().unwrap();
        let registry = SftpRegistry::default();
        let _ = registry.connect("h".into(), "/bin/sh", &counting_ssh(d.path()), &[], Duration::from_millis(2500));
        assert!(attempts(d.path()) >= 2, "a positive timeout must retry");
    }

    /// "failed to fill whole buffer" explained nothing; ssh's own words do.
    #[test]
    fn the_error_carries_what_ssh_said_and_the_environment_reaches_it() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("env.txt");
        let script = format!(
            "printf %s \"$BHT_PROBE\" > '{}'; echo 'Warning: Permanently added x to known hosts.' >&2; \
             echo 'root@h: Permission denied (publickey,password).' >&2; exit 255",
            out.display()
        );
        let registry = SftpRegistry::default();
        let err = registry
            .connect("h".into(), "/bin/sh", &sh(&script), &[("BHT_PROBE".into(), "from-the-app".into())], Duration::ZERO)
            .err()
            .expect("must fail");
        assert!(err.contains("ssh said"), "{err}");
        assert!(err.contains("Permission denied (publickey,password)"), "{err}");
        assert!(!err.contains("Permanently added"), "harmless notice must be dropped: {err}");
        assert_eq!(std::fs::read_to_string(out).unwrap(), "from-the-app");
    }

    #[test]
    fn what_ssh_said_is_cleaned_and_capped() {
        let said = Mutex::new(format!(
            "\x1b[31mHost key verification failed.\x1b[0m\r\n{}\n",
            "x".repeat(900)
        ));
        let msg = with_ssh_said("base".into(), &said);
        assert!(msg.starts_with("base — ssh said: "));
        assert!(!msg.contains('\x1b') && !msg.contains('\r'));
        assert!(msg.chars().count() < 300, "capped for a status line: {}", msg.len());
        assert_eq!(with_ssh_said("base".into(), &Mutex::new(String::new())), "base");
    }

    fn wait_until_flag(dir: &std::path::Path) -> Vec<String> {
        sh(&format!("test -f '{}'", dir.join("up").display()))
    }

    /// The fix: wait on the local socket for the terminal's login instead of
    /// dialling the server.
    #[test]
    fn the_wait_ends_as_soon_as_the_terminals_connection_exists() {
        let d = tempfile::tempdir().unwrap();
        let flag = d.path().join("up");
        let registry = SftpRegistry::default();
        let args = wait_until_flag(d.path());
        let creator = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1800));
            std::fs::write(flag, "1").unwrap();
        });
        let started = Instant::now();
        registry.wait_for_master("h", "/bin/sh", &args, Duration::from_secs(30)).unwrap();
        let took = started.elapsed();
        creator.join().unwrap();
        assert!(took >= Duration::from_millis(1500), "returned before the master existed: {took:?}");
        assert!(took < Duration::from_secs(6), "did not notice the master promptly: {took:?}");
        assert!(registry.waiting.lock().unwrap().is_empty(), "wait must clean up after itself");
    }

    #[test]
    fn the_wait_gives_up_at_its_deadline_with_a_useful_message() {
        let d = tempfile::tempdir().unwrap();
        let registry = SftpRegistry::default();
        let err = registry
            .wait_for_master("h", "/bin/sh", &wait_until_flag(d.path()), Duration::from_millis(1500))
            .unwrap_err();
        assert!(err.contains("did not finish logging in"), "{err}");
        assert!(registry.waiting.lock().unwrap().is_empty());
    }

    fn start_wait(registry: &Arc<SftpRegistry>, host: &str, args: Vec<String>) -> std::thread::JoinHandle<Result<(), String>> {
        let (registry, host) = (Arc::clone(registry), host.to_string());
        let handle = std::thread::spawn(move || registry.wait_for_master(&host, "/bin/sh", &args, Duration::from_secs(60)));
        std::thread::sleep(Duration::from_millis(600)); // let it register
        handle
    }

    /// Closing the terminal (prune/disconnect) must free the waiting thread —
    /// otherwise every abandoned connect leaves a poller running for 15 minutes.
    #[test]
    fn closing_the_terminal_cancels_the_wait() {
        let d = tempfile::tempdir().unwrap();
        let registry = Arc::new(SftpRegistry::default());
        let waiter = start_wait(&registry, "h1", wait_until_flag(d.path()));
        let other = start_wait(&registry, "h2", wait_until_flag(d.path()));
        let started = Instant::now();
        registry.prune(&["h2".to_string()]); // h1's terminal is gone, h2's is not
        assert!(waiter.join().unwrap().unwrap_err().contains("stopped waiting"));
        assert!(started.elapsed() < Duration::from_secs(5));
        // h2 is still waiting; disconnecting it ends that one too.
        registry.disconnect("h2");
        assert!(other.join().unwrap().is_err());
    }

    #[test]
    fn a_newer_wait_for_the_same_host_supersedes_the_older() {
        let d = tempfile::tempdir().unwrap();
        let registry = Arc::new(SftpRegistry::default());
        let first = start_wait(&registry, "h", wait_until_flag(d.path()));
        let second = start_wait(&registry, "h", wait_until_flag(d.path()));
        assert!(first.join().unwrap().unwrap_err().contains("stopped waiting"));
        // The newer one is unaffected and finishes when the master appears.
        std::fs::write(d.path().join("up"), "1").unwrap();
        assert!(second.join().unwrap().is_ok());
    }

    /// A wedged `ssh -O check` must not pin the waiting thread.
    #[test]
    fn a_hung_check_is_cut_off_and_counts_as_not_up() {
        let started = Instant::now();
        assert!(!SftpRegistry::master_is_up("/bin/sh", &sh("sleep 30")));
        assert!(started.elapsed() < Duration::from_secs(9), "check had no deadline");
    }

    #[test]
    fn a_wait_can_be_cancelled_directly() {
        let d = tempfile::tempdir().unwrap();
        let registry = Arc::new(SftpRegistry::default());
        let waiter = start_wait(&registry, "h", wait_until_flag(d.path()));
        registry.cancel_wait("h");
        assert!(waiter.join().unwrap().unwrap_err().contains("stopped waiting"));
    }
}
