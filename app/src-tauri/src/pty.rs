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
use tauri::ipc::Channel;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PtyEvent {
    /// Base64 because the payload is raw bytes, not text: a UTF-8 sequence
    /// can straddle two reads, and decoding half of one would corrupt the
    /// stream. The webview decodes and hands xterm.js the bytes.
    Output { data: String },
    /// A clean logout (0) closes the tab; anything else kept it open with a
    /// Reconnect button in the Swift build, and should here too.
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
    pub fn spawn(
        &self,
        id: String,
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        on_event: Channel<PtyEvent>,
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
                        let msg = PtyEvent::Output { data: B64.encode(&buf[..n]) };
                        if on_event.send(msg).is_err() {
                            break; // the webview went away
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = on_event.send(PtyEvent::Exit { code: None });
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
