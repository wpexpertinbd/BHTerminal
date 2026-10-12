//! Running port forwards.
//!
//! Each enabled rule is its own `ssh -N -L/-R/-D`, started on demand and
//! tracked here rather than in the view — the same reason the terminal
//! sessions are: a forward must not die because a panel was re-rendered, and
//! must not survive the app either.
//!
//! A forward joins the terminal's shared connection when one is live, so it
//! needs no second login. When there is none, ssh is run with BatchMode so it
//! FAILS instead of blocking forever on a password prompt that has no terminal
//! to appear in — and the failure is reported with ssh's own stderr, which is
//! the only thing that explains why.

use bhterm_core::{Host, TunnelRule};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};

struct Running {
    child: Child,
    /// The last lines ssh wrote, kept so a forward that dies can say why.
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
pub struct TunnelRegistry {
    running: Mutex<HashMap<String, Running>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub rule_id: String,
    pub running: bool,
    /// Set once the process has exited: why it stopped.
    pub message: Option<String>,
}

impl TunnelRegistry {
    pub fn start(
        &self,
        host: &Host,
        jump: Option<&Host>,
        rule: &TunnelRule,
        socket_dir: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let key = rule.id.to_string();
        let mut running = self.running.lock().unwrap();
        if let Some(existing) = running.get_mut(&key) {
            match existing.child.try_wait() {
                Ok(None) => return Ok(()), // already up
                _ => {
                    running.remove(&key);
                }
            }
        }

        let cm = socket_dir.map(|d| bhterm_core::ssh::ControlMaster { socket_dir: d });
        let mut args = bhterm_core::ssh::build_tunnel(host, jump, rule, cm.as_ref())
            .map_err(|e| e.to_string())?;
        // Without this a forward on a host with no live session waits on a
        // password prompt nobody can see, and looks like a hang.
        args.insert(0, "BatchMode=yes".into());
        args.insert(0, "-o".into());

        let mut child = crate::proc::command("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start ssh: {e}"))?;

        let log = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut log = log.lock().unwrap();
                    // Bounded: a flapping forward can log forever.
                    if log.len() == 20 {
                        log.remove(0);
                    }
                    log.push(line);
                }
            });
        }

        running.insert(key, Running { child, log });
        Ok(())
    }

    pub fn stop(&self, rule_id: &str) {
        if let Some(mut r) = self.running.lock().unwrap().remove(rule_id) {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    /// Status for the rules asked about, reaping any that have exited so a
    /// forward that died reports its reason instead of looking alive.
    pub fn status(&self, rule_ids: &[String]) -> Vec<TunnelStatus> {
        let mut running = self.running.lock().unwrap();
        rule_ids
            .iter()
            .map(|id| match running.get_mut(id) {
                None => TunnelStatus { rule_id: id.clone(), running: false, message: None },
                Some(r) => match r.child.try_wait() {
                    Ok(None) => TunnelStatus { rule_id: id.clone(), running: true, message: None },
                    Ok(Some(status)) => {
                        let log = r.log.lock().unwrap().join(" ");
                        let message = if log.trim().is_empty() {
                            format!("ssh exited ({status})")
                        } else {
                            log
                        };
                        running.remove(id);
                        TunnelStatus { rule_id: id.clone(), running: false, message: Some(message) }
                    }
                    Err(e) => {
                        running.remove(id);
                        TunnelStatus {
                            rule_id: id.clone(),
                            running: false,
                            message: Some(e.to_string()),
                        }
                    }
                },
            })
            .collect()
    }

    /// Every forward, killed. Called on quit — ssh children would otherwise
    /// outlive the app that started them.
    pub fn stop_all(&self) {
        let mut running = self.running.lock().unwrap();
        for (_, mut r) in running.drain() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }
}

pub type SharedTunnels = Arc<TunnelRegistry>;
