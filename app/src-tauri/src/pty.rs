//! Live terminal sessions.
//!
//! ⭐ The sessions live HERE, in Rust state that lasts as long as the app —
//! never in the UI layer. The Swift build learned this the hard way: it kept
//! each session inside the view that displayed it, so switching tabs tore the
//! view down and killed that server's ssh, and returning showed a brand-new
//! login. A webview is even more volatile than a SwiftUI view (a reload
//! discards everything), so a pane is closed only when the user closes it.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use bhterm_core::prompt::{what_is_wanted, Wanted};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PtyEvent {
    /// Base64 because the payload is raw bytes, not text: a UTF-8 sequence
    /// can straddle two reads, and decoding half of one would corrupt the
    /// stream. The webview decodes and hands xterm.js the bytes.
    Output { data: String },
    /// A clean logout (0) closes the tab; anything else keeps it open with a
    /// Reconnect button, as the Swift build learned to do.
    Exit { code: Option<i32> },
}

/// Secrets this pane may type in answer to ssh's own prompts.
#[derive(Default, Clone)]
pub struct AuthSecrets {
    pub password: Option<String>,
    pub passphrase: Option<String>,
    /// True only for a saved SSH host. Turns on the "sent the saved
    /// password" / "no saved password" lines — and keeps them OFF a local
    /// shell, where a `sudo` prompt is none of BHTerminal's business.
    pub notify: bool,
    /// The host logs in with the ssh agent, so the editor has no password box
    /// to point at: the hint says to switch the login method first.
    pub agent: bool,
}

impl AuthSecrets {
    fn is_empty(&self) -> bool {
        self.password.is_none() && self.passphrase.is_none() && !self.notify
    }
}

/// The ONE write handle of a pty, shared by the keyboard and the auto-fill.
///
/// ⚠️ portable-pty hands the writer out once — "It is invalid to take the
/// writer more than once" — and a second `take_writer()` returns an error.
/// The auto-fill used to ask for its own, got `None` from `.ok()`, and so
/// NEVER typed anything: every saved password and passphrase was ignored, with
/// no error anywhere. Sharing the one handle is the only way both can write.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// The log file a session is writing to, shared with its reader thread so
/// logging can be switched on and off while the session runs.
type LogSink = Arc<Mutex<Option<LogFile>>>;

struct LogFile {
    file: std::fs::File,
    path: PathBuf,
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    log: LogSink,
}

#[derive(Default)]
pub struct PtyRegistry {
    sessions: Mutex<HashMap<String, Session>>,
}

impl PtyRegistry {
    /// `on_event` returns false when the receiver has gone away, which stops
    /// the reader thread. Taking a plain callback rather than a Tauri Channel
    /// keeps this testable without a running app — the command layer wraps the
    /// Channel.
    #[allow(clippy::too_many_arguments)] // the pty genuinely needs all of them
    pub fn spawn(
        &self,
        id: String,
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        auth: AuthSecrets,
        on_event: impl Fn(PtyEvent) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("couldn't open a pty: {e}"))?;

        let mut cmd = CommandBuilder::new(program);
        for a in args {
            cmd.arg(a);
        }
        // ⚠️ TERM must match `xterm*`: that is the glob distro shell configs
        // check before they publish the working directory in the window
        // title, and the title is where cwd-follow reads it from. Lose this
        // and the SFTP pane silently stops following the terminal.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("couldn't start {program}: {e}"))?;
        // The slave must be dropped in the parent, or the reader never sees
        // EOF when the child exits and the pane hangs open forever.
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("couldn't read from the pty: {e}"))?;
        let writer: SharedWriter = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| format!("couldn't write to the pty: {e}"))?,
        ));
        // The same handle, for answering ssh's auth prompts.
        let fill_writer = if auth.is_empty() { None } else { Some(Arc::clone(&writer)) };

        let log: LogSink = Arc::new(Mutex::new(None));
        let log_for_reader = Arc::clone(&log);
        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session { master: pair.master, writer, child, log },
        );

        // One blocking reader per pane. A pty read has no async story worth
        // the complexity here, and a terminal that stalls its own output is
        // worse than a thread.
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut filler = SecretFiller::new(auth, fill_writer);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let notes = filler.observe(&buf[..n]);
                        // Only what the server SENT is logged, never what was
                        // typed or filled in — a transcript must not end up
                        // holding the password that opened the session.
                        if let Some(sink) = log_for_reader.lock().unwrap().as_mut() {
                            // A log that cannot be written is dropped rather
                            // than allowed to stall the terminal.
                            let _ = sink.file.write_all(&buf[..n]);
                        }
                        if !on_event(PtyEvent::Output { data: B64.encode(&buf[..n]) }) {
                            break; // the receiver went away
                        }
                        // After the server's own output, so a line reads in the
                        // order things happened. Display only: never logged,
                        // never sent to the server.
                        for note in notes {
                            if !on_event(PtyEvent::Output { data: B64.encode(note.as_bytes()) }) {
                                return;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            on_event(PtyEvent::Exit { code: None });
        });

        Ok(())
    }

    pub fn write(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id).ok_or("no such terminal")?;
        let mut writer = session.writer.lock().unwrap();
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id).ok_or("no such terminal")?;
        session
            .master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())
    }

    /// Explicit close is the ONLY thing that ends a session — see the note at
    /// the top of this file.
    pub fn close(&self, id: &str) {
        if let Some(mut session) = self.sessions.lock().unwrap().remove(id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }

    pub fn close_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }

    /// Starts appending this session's output to `path`. Appends rather than
    /// truncates: reconnecting to a host should extend its transcript, not
    /// erase what was already captured.
    pub fn start_logging(&self, id: &str, path: &Path) -> Result<(), String> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(id).ok_or("no such terminal")?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        // A transcript holds whatever the session printed, so it is created
        // owner-only — not merely moved there afterwards, which would leave a
        // window where another local account could open it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;

        *session.log.lock().unwrap() = Some(LogFile { file, path: path.to_path_buf() });
        Ok(())
    }

    pub fn stop_logging(&self, id: &str) {
        if let Some(session) = self.sessions.lock().unwrap().get(id) {
            // Dropping the handle flushes and closes it.
            *session.log.lock().unwrap() = None;
        }
    }

    /// Where this session is logging, if it is.
    pub fn log_path(&self, id: &str) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();
        let log = sessions.get(id)?.log.lock().unwrap();
        log.as_ref().map(|l| l.path.display().to_string())
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(id)
    }
}

pub type SharedRegistry = Arc<PtyRegistry>;

/// Types a stored secret when ssh asks for it, and says so.
///
/// It watches only the tail of the stream, and only until the credentials are
/// spent or the window closes — holding a password in memory for the life of
/// a session that no longer needs it is pointless risk.
///
/// A saved PASSWORD is typed at most ONCE per connection. A wrong one would
/// otherwise be retried at every re-prompt — burning the server's three
/// attempts, tripping fail2ban on a live machine, and leaving the human with
/// nothing left to type. After one rejection the human takes over.
struct SecretFiller {
    auth: AuthSecrets,
    writer: Option<SharedWriter>,
    tail: Vec<u8>,
    /// A passphrase can legitimately be asked more than once (a jump host and
    /// the target each load the key); capped so a wrong one cannot loop.
    passphrase_left: u8,
    password_left: u8,
    sent_password: bool,
    told_no_password: bool,
    started: Instant,
}

impl SecretFiller {
    /// Enough to hold a prompt line; prompts are short and we only need the
    /// last one. Generous, because a ConPTY redraw puts escape sequences
    /// around it.
    const WINDOW: usize = 1024;
    /// After this, auth has either succeeded or is waiting on a human.
    const GIVE_UP_AFTER: Duration = Duration::from_secs(180);
    /// Typed this long after the prompt is SEEN. ConPTY paints the prompt a
    /// beat before ssh is reading the console, and a key sent into that gap
    /// can be dropped.
    const SETTLE: Duration = Duration::from_millis(120);

    fn new(auth: AuthSecrets, writer: Option<SharedWriter>) -> Self {
        SecretFiller {
            auth,
            writer,
            tail: Vec::new(),
            passphrase_left: 2,
            password_left: 1,
            sent_password: false,
            told_no_password: false,
            started: Instant::now(),
        }
    }

    fn done(&self) -> bool {
        self.writer.is_none()
    }

    /// Display-only lines for the terminal, in dim text.
    fn note(text: &str) -> String {
        format!("\r\n\x1b[2m[BHTerminal] {text}\x1b[0m\r\n")
    }

    fn type_in(&self, secret: &str) {
        std::thread::sleep(Self::SETTLE);
        if let Some(w) = &self.writer {
            if let Ok(mut w) = w.lock() {
                let _ = w.write_all(secret.as_bytes());
                let _ = w.write_all(b"\r");
                let _ = w.flush();
            }
        }
    }

    fn observe(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut notes = Vec::new();
        if self.done() {
            return notes;
        }
        if self.started.elapsed() > Self::GIVE_UP_AFTER {
            self.forget();
            return notes;
        }

        self.tail.extend_from_slice(chunk);
        if self.tail.len() > Self::WINDOW {
            let drop = self.tail.len() - Self::WINDOW;
            self.tail.drain(..drop);
        }
        let text = String::from_utf8_lossy(&self.tail).into_owned();

        // The server turned our saved password down: say so, once, and leave
        // the retry to the human.
        if self.sent_password && bhterm_core::prompt::strip_ansi(&text).to_lowercase().contains("permission denied") {
            self.sent_password = false;
            if self.auth.notify {
                notes.push(Self::note(
                    "The server rejected the saved password. Type the right one, or fix it under Edit host (the username matters too).",
                ));
            }
        }

        // Never answer a host-key question or a 2FA code with a stored
        // secret — only the user can answer those.
        match what_is_wanted(&text) {
            Some(Wanted::Passphrase) if self.passphrase_left > 0 => {
                if let Some(secret) = self.auth.passphrase.clone() {
                    self.type_in(&secret);
                    self.passphrase_left -= 1;
                    self.tail.clear();
                    if self.auth.notify {
                        notes.push(Self::note("Sent the saved key passphrase."));
                    }
                }
            }
            Some(Wanted::Password) if self.password_left > 0 => {
                if let Some(secret) = self.auth.password.clone() {
                    self.type_in(&secret);
                    self.password_left -= 1;
                    self.sent_password = true;
                    self.tail.clear();
                    if self.auth.notify {
                        notes.push(Self::note("Sent the saved password."));
                    }
                } else if self.auth.notify && !self.told_no_password {
                    self.told_no_password = true;
                    notes.push(Self::note(if self.auth.agent {
                        "No saved password. Type it here — or set this host's login to Password under Edit host to save it."
                    } else {
                        "No saved password for this host. Type it here, or save it under Edit host."
                    }));
                }
            }
            _ => {}
        }

        // Nothing left to offer, and nothing left to report.
        if self.passphrase_left == 0 && self.password_left == 0 && !self.sent_password {
            self.forget();
        }
        notes
    }

    fn forget(&mut self) {
        self.auth = AuthSecrets::default();
        self.writer = None;
        self.tail.clear();
        self.passphrase_left = 0;
        self.password_left = 0;
        self.sent_password = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Runs `script` as a stand-in for ssh and returns everything it printed.
    /// `then` is typed by the "human" after the given delay.
    fn run_fake_ssh(script: &str, auth: AuthSecrets, then: Option<(u64, &str)>) -> String {
        let registry = PtyRegistry::default();
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        registry
            .spawn(
                "fake".into(),
                "/bin/sh",
                &["-c".to_string(), script.to_string()],
                100,
                30,
                auth,
                move |e| tx.send(e).is_ok(),
            )
            .unwrap();
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        let mut typed = then.is_none();
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PtyEvent::Output { data }) => out.extend(B64.decode(data).unwrap()),
                Ok(PtyEvent::Exit { .. }) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let (false, Some((after, input))) = (typed, then) {
                if start.elapsed() > Duration::from_millis(after) {
                    typed = true;
                    registry.write("fake", input.as_bytes()).unwrap();
                }
            }
            if start.elapsed() > Duration::from_secs(15) {
                break;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn pw(secret: &str) -> AuthSecrets {
        AuthSecrets { password: Some(secret.into()), notify: true, ..AuthSecrets::default() }
    }

    /// The bug that made every saved password a no-op: the auto-fill asked
    /// the pty for a SECOND writer, the library refuses that, and `.ok()`
    /// turned the refusal into silence. The password must reach the program,
    /// byte for byte, followed by Enter.
    #[test]
    fn the_saved_password_is_typed_exactly_when_ssh_asks() {
        // Spaces, quotes, `$`, a backslash and Bengali: nothing may be
        // mangled on the way in.
        let secret = "P@ss w0rd!\"'$HOME\\ পাসওয়ার্ড";
        let out = run_fake_ssh(
            "stty -echo; printf \"root@h's password: \"; IFS= read -r p; printf '\\nGOT:[%s]\\n' \"$p\"",
            pw(secret),
            None,
        );
        assert!(out.contains(&format!("GOT:[{secret}]")), "the secret did not arrive intact: {out:?}");
        assert!(out.contains("Sent the saved password"), "no visible confirmation: {out:?}");
    }

    /// ConPTY (Windows) re-draws the screen, so the prompt arrives wrapped in
    /// a window-title sequence and cursor/visibility codes. It must still be
    /// recognised.
    #[test]
    fn a_prompt_wrapped_in_windows_control_codes_is_still_answered() {
        let out = run_fake_ssh(
            "stty -echo; printf '\\033[?25l\\033]0;C:\\\\ssh.exe\\007\\033[25;1Hroot@h'\"'\"'s password: \\033[25;28H\\033[?25h'; \
             IFS= read -r p; printf '\\nGOT:[%s]\\n' \"$p\"",
            pw("conpty-secret"),
            None,
        );
        assert!(out.contains("GOT:[conpty-secret]"), "control codes hid the prompt: {out:?}");
    }

    /// A wrong saved password is tried ONCE. Retrying at every re-prompt
    /// burns the server's attempts (and trips fail2ban on a live machine),
    /// and leaves nothing for the human to type.
    #[test]
    fn a_rejected_saved_password_is_not_retried_and_the_human_can_type() {
        let out = run_fake_ssh(
            "stty -echo; \
             printf \"root@h's password: \"; IFS= read -r a; printf '\\nGOT1:[%s]\\nPermission denied, please try again.\\n' \"$a\"; \
             printf \"root@h's password: \"; IFS= read -r b; printf '\\nGOT2:[%s]\\n' \"$b\"",
            pw("wrong-saved"),
            Some((2500, "typed-by-human\r")),
        );
        assert!(out.contains("GOT1:[wrong-saved]"), "{out:?}");
        assert!(out.contains("GOT2:[typed-by-human]"), "the second prompt must be left to the human: {out:?}");
        assert!(!out.contains("GOT2:[wrong-saved]"), "the wrong password was retried: {out:?}");
        assert!(out.contains("server rejected the saved password"), "no explanation shown: {out:?}");
    }

    #[test]
    fn with_no_saved_password_it_says_so_and_types_nothing() {
        let out = run_fake_ssh(
            "stty -echo; printf \"root@h's password: \"; IFS= read -r -t 2 p; printf '\\nGOT:[%s]\\n' \"$p\"",
            AuthSecrets { notify: true, ..AuthSecrets::default() },
            None,
        );
        assert!(out.contains("GOT:[]"), "something was typed with nothing saved: {out:?}");
        assert!(out.contains("No saved password for this host"), "{out:?}");
    }

    #[test]
    fn an_agent_host_is_told_how_to_save_a_password() {
        let out = run_fake_ssh(
            "stty -echo; printf \"root@h's password: \"; IFS= read -r -t 2 p; printf '\\nGOT:[%s]\\n' \"$p\"",
            AuthSecrets { notify: true, agent: true, ..AuthSecrets::default() },
            None,
        );
        assert!(out.contains("GOT:[]"), "{out:?}");
        assert!(out.contains("set this host's login to Password"), "{out:?}");
    }

    /// A local shell is none of BHTerminal's business: a `sudo` prompt there
    /// must not get a saved password, nor a comment.
    #[test]
    fn a_local_shell_prompt_is_never_answered_or_commented_on() {
        let out = run_fake_ssh(
            "stty -echo; printf '[sudo] password for me: '; IFS= read -r -t 2 p; printf '\\nGOT:[%s]\\n' \"$p\"",
            AuthSecrets::default(),
            None,
        );
        assert!(out.contains("GOT:[]"), "{out:?}");
        assert!(!out.contains("[BHTerminal]"), "{out:?}");
    }

    /// A password prompt that shows up long after login (a remote `sudo`)
    /// must not receive the saved password a second time.
    #[test]
    fn the_saved_password_is_not_offered_again_at_a_later_prompt() {
        let out = run_fake_ssh(
            "stty -echo; \
             printf \"root@h's password: \"; IFS= read -r a; printf '\\nGOT1:[%s]\\nWelcome\\n' \"$a\"; \
             printf '[sudo] password for root: '; IFS= read -r -t 2 b; printf '\\nGOT2:[%s]\\n' \"$b\"",
            pw("login-secret"),
            None,
        );
        assert!(out.contains("GOT1:[login-secret]"), "{out:?}");
        assert!(out.contains("GOT2:[]"), "the password was typed into a later prompt: {out:?}");
    }

    /// Proves the pty round-trip end to end: a child is spawned, its output
    /// reaches the callback as bytes, and EOF produces an Exit. If this
    /// breaks, every terminal in the app is dead.
    #[test]
    fn spawns_a_command_and_streams_its_output() {
        let registry = PtyRegistry::default();
        let (tx, rx) = mpsc::channel::<PtyEvent>();

        registry
            .spawn(
                "t1".into(),
                "/bin/sh",
                &["-c".to_string(), "echo hello-from-pty".to_string()],
                80,
                24,
                AuthSecrets::default(),
                move |e| tx.send(e).is_ok(),
            )
            .expect("spawn");

        let mut seen = String::new();
        let mut exited = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Output { data }) => {
                    let bytes = B64.decode(&data).expect("valid base64");
                    seen.push_str(&String::from_utf8_lossy(&bytes));
                }
                Ok(PtyEvent::Exit { .. }) => {
                    exited = true;
                    break;
                }
                Err(_) => break,
            }
        }

        assert!(seen.contains("hello-from-pty"), "got: {seen:?}");
        assert!(exited, "the reader must report EOF when the child exits");
        registry.close("t1");
    }

    #[test]
    fn a_closed_session_is_gone_and_writing_to_it_fails() {
        let registry = PtyRegistry::default();
        registry
            .spawn(
                "t2".into(),
                "/bin/sh",
                &["-c".into(), "sleep 30".into()],
                80,
                24,
                AuthSecrets::default(),
                |_| true,
            )
            .expect("spawn");
        assert!(registry.is_running("t2"));

        registry.close("t2");
        assert!(!registry.is_running("t2"));
        // Closing is the only thing that ends a session, and once closed the
        // id must not silently accept input.
        assert!(registry.write("t2", b"x").is_err());
        assert!(registry.resize("t2", 100, 30).is_err());
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;

    /// The transcript must hold what the session PRINTED and nothing that was
    /// typed into it — a log holding the password that opened the session
    /// would be worse than no log at all.
    #[test]
    fn a_transcript_captures_output_but_never_input() {
        let dir = std::env::temp_dir().join(format!("bhterm-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.log");

        let registry = PtyRegistry::default();
        let (tx, rx) = std::sync::mpsc::channel();
        registry
            .spawn(
                "log1".into(),
                "/bin/sh",
                &["-c".into(), "cat".into()],
                80,
                24,
                AuthSecrets::default(),
                move |e| tx.send(e).is_ok(),
            )
            .expect("spawn");

        registry.start_logging("log1", &path).expect("start logging");
        assert_eq!(registry.log_path("log1").as_deref(), Some(path.display().to_string().as_str()));

        // `cat` echoes, so this arrives back as OUTPUT — which is what proves
        // the log is capturing the stream rather than the keystrokes.
        registry.write("log1", b"hello-from-the-shell\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while Instant::now() < deadline && !seen.contains("hello-from-the-shell") {
            if let Ok(PtyEvent::Output { data }) = rx.recv_timeout(Duration::from_millis(250)) {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
            }
        }
        registry.stop_logging("log1");
        registry.close("log1");

        let logged = std::fs::read_to_string(&path).expect("the log file exists");
        assert!(logged.contains("hello-from-the-shell"), "output was not logged: {logged:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "a transcript must not be readable by other accounts");
        }

        // Stopping means stopping: nothing more is appended.
        let size_after_stop = std::fs::metadata(&path).unwrap().len();
        assert!(registry.log_path("log1").is_none());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size_after_stop);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logging_an_unknown_pane_is_an_error_not_a_silent_no_op() {
        let registry = PtyRegistry::default();
        let path = std::env::temp_dir().join("bhterm-nope.log");
        assert!(registry.start_logging("missing", &path).is_err());
        assert!(!path.exists(), "no file is created for a pane that does not exist");
    }
}
