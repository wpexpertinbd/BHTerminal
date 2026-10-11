//! Live terminal sessions.
//!
//! ⭐ The sessions live HERE, in Rust state that lasts as long as the app —
//! never in the UI layer. The Swift build learned this the hard way: it kept
//! each session inside the view that displayed it, so switching tabs tore the
//! view down and killed that server's ssh, and returning showed a brand-new
//! login. A webview is even more volatile than a SwiftUI view (a reload
//! discards everything), so a pane is closed only when the user closes it.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

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

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
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
    pub fn spawn(
        &self,
        id: String,
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
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
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("couldn't write to the pty: {e}"))?;

        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session { master: pair.master, writer, child },
        );

        // One blocking reader per pane. A pty read has no async story worth
        // the complexity here, and a terminal that stalls its own output is
        // worse than a thread.
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if !on_event(PtyEvent::Output { data: B64.encode(&buf[..n]) }) {
                            break; // the receiver went away
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
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(id).ok_or("no such terminal")?;
        session.writer.write_all(data).map_err(|e| e.to_string())?;
        session.writer.flush().map_err(|e| e.to_string())
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

    pub fn is_running(&self, id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(id)
    }
}

pub type SharedRegistry = Arc<PtyRegistry>;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::mpsc;
    use std::time::Duration;

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
            .spawn("t2".into(), "/bin/sh", &["-c".into(), "sleep 30".into()], 80, 24, |_| true)
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
