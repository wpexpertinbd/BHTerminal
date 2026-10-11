//! Turning a saved Host into argv for the real `ssh` binary — plus the
//! argument-injection guard that gates it.
//!
//! Shelling out to the system `ssh` (rather than reimplementing SSH) is what
//! gives us `~/.ssh/config`, the agent, ProxyJump, 2FA and host-key TOFU for
//! free, and it works on all three platforms: Windows 10+ ships OpenSSH.

use crate::model::{AuthMethod, Host, TunnelKind, TunnelRule};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SshSafetyError {
    #[error("“{0}” is not a valid hostname. Hostnames may only contain letters, digits, and . - _ : % [ ] and must not start with “-”.")]
    UnsafeHostname(String),
    #[error("“{0}” is not a valid username. Usernames may only contain letters, digits, and . - _ and must not start with “-”.")]
    UnsafeUsername(String),
}

/// Guards against argument injection into `ssh`.
///
/// The destination operand (`user@host`) is the one argv element ssh will
/// re-read as an OPTION when it begins with “-”: a username of
/// `-oProxyCommand=…` makes ssh run an arbitrary command through the shell
/// before authentication. Jump-host values ride inside `-J` and feed an inner
/// ssh, so they get the same treatment. This is an allow-list on purpose —
/// for a credential-holding tool, refusing a weird value beats trying to
/// escape it.
pub fn is_valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:%[]".contains(c))
}

pub fn is_valid_username(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
}

pub fn validate(host: &Host, jump: Option<&Host>) -> Result<(), SshSafetyError> {
    if !is_valid_hostname(&host.hostname) {
        return Err(SshSafetyError::UnsafeHostname(host.hostname.clone()));
    }
    if !is_valid_username(&host.username) {
        return Err(SshSafetyError::UnsafeUsername(host.username.clone()));
    }
    if let Some(j) = jump {
        if !is_valid_hostname(&j.hostname) {
            return Err(SshSafetyError::UnsafeHostname(j.hostname.clone()));
        }
        if !is_valid_username(&j.username) {
            return Err(SshSafetyError::UnsafeUsername(j.username.clone()));
        }
    }
    Ok(())
}

/// Where the shared-connection socket lives, and whether this platform has
/// one at all.
///
/// ⚠️ Windows OpenSSH does **not** implement ControlMaster/ControlPath, so on
/// Windows there is no shared connection: SFTP opens its own, authenticating
/// again (the stored secret is auto-filled, so it stays invisible to the
/// user). Passing those options there would make ssh fail outright, so they
/// are omitted rather than pointed somewhere harmless.
#[derive(Debug, Clone)]
pub struct ControlMaster<'a> {
    /// Directory for the socket. The shell supplies it (0700, under the
    /// user's data dir) — the core never guesses a path.
    pub socket_dir: &'a Path,
}

pub fn multiplexing_supported() -> bool {
    !cfg!(windows)
}

/// Keepalives are what let a dropped network actually END the session instead
/// of wedging it: without them the master survives with a dead TCP connection
/// and every later session — including a manual reconnect — multiplexes onto
/// the corpse and hangs until the kernel's TCP timeout.
fn shared_options(host: &Host, jump: Option<&Host>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-o".into(), "ServerAliveInterval=15".into(),
        "-o".into(), "ServerAliveCountMax=3".into(),
        "-o".into(), "TCPKeepAlive=yes".into(),
        "-o".into(), "ConnectTimeout=20".into(),
    ];
    if host.port != 22 {
        args.push("-p".into());
        args.push(host.port.to_string());
    }
    if let AuthMethod::PrivateKey { path } = &host.auth_method {
        args.push("-i".into());
        args.push(expand_tilde(path));
    }
    if let Some(j) = jump {
        args.push("-J".into());
        args.push(jump_spec(j));
    }
    args
}

pub fn jump_spec(host: &Host) -> String {
    if host.port != 22 {
        format!("{}@{}:{}", host.username, host.hostname, host.port)
    } else {
        format!("{}@{}", host.username, host.hostname)
    }
}

pub fn destination(host: &Host) -> String {
    format!("{}@{}", host.username, host.hostname)
}

/// `~` is the user's own shell convention, not something `ssh -i` expands.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return format!("{}/{}", home.trim_end_matches('/'), rest);
        }
    }
    path.to_string()
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
}

fn control_args(cm: Option<&ControlMaster>, as_master: bool) -> Vec<String> {
    let Some(cm) = cm else { return Vec::new() };
    if !multiplexing_supported() {
        return Vec::new();
    }
    // `%C` is ssh's own hash of (localhost, remotehost, port, user): short,
    // and identical for the terminal and the SFTP channel, so they share one
    // master.
    let path = cm.socket_dir.join("%C");
    let mut args = vec!["-o".to_string(), format!("ControlPath={}", path.display())];
    if as_master {
        args.push("-o".into());
        args.push("ControlMaster=auto".into());
        args.push("-o".into());
        args.push("ControlPersist=120".into());
    }
    args
}

/// argv for the interactive terminal.
pub fn build_terminal(
    host: &Host,
    jump: Option<&Host>,
    cm: Option<&ControlMaster>,
) -> Result<Vec<String>, SshSafetyError> {
    validate(host, jump)?;
    let mut args = shared_options(host, jump);
    args.extend(control_args(cm, true));
    // "--" ends option parsing so the destination can never be re-read as an
    // option — defence in depth behind the allow-list above.
    args.push("--".into());
    args.push(destination(host));
    Ok(args)
}

/// argv for the SFTP subsystem, reusing the terminal's connection where the
/// platform has one. `BatchMode=yes` means it never blocks on a prompt it has
/// no tty for: if the master isn't up yet it fails fast and the caller retries.
pub fn build_sftp(
    host: &Host,
    jump: Option<&Host>,
    cm: Option<&ControlMaster>,
) -> Result<Vec<String>, SshSafetyError> {
    validate(host, jump)?;
    let mut args = shared_options(host, jump);
    args.extend(control_args(cm, false));
    args.push("-o".into());
    args.push("BatchMode=yes".into());
    args.push("-s".into());
    args.push("--".into());
    args.push(destination(host));
    args.push("sftp".into());
    Ok(args)
}

/// argv for a port-forward, run as its own background ssh.
pub fn build_tunnel(
    host: &Host,
    jump: Option<&Host>,
    rule: &TunnelRule,
) -> Result<Vec<String>, SshSafetyError> {
    validate(host, jump)?;
    let mut args = shared_options(host, jump);
    let spec = match rule.kind {
        TunnelKind::Local => format!(
            "{}:{}:{}:{}",
            rule.listen_host, rule.listen_port, rule.dest_host, rule.dest_port
        ),
        TunnelKind::Remote => format!(
            "{}:{}:{}:{}",
            rule.listen_host, rule.listen_port, rule.dest_host, rule.dest_port
        ),
        TunnelKind::Dynamic => format!("{}:{}", rule.listen_host, rule.listen_port),
    };
    args.push(
        match rule.kind {
            TunnelKind::Local => "-L",
            TunnelKind::Remote => "-R",
            TunnelKind::Dynamic => "-D",
        }
        .into(),
    );
    args.push(spec);
    // -N: no remote command, the forward is the whole point.
    args.push("-N".into());
    args.push("--".into());
    args.push(destination(host));
    Ok(args)
}

/// Force-closes the shared connection so the next connect builds a new one.
///
/// A user-initiated reconnect must not reuse the existing master: after a
/// network drop the master can still be alive locally (its socket answers, so
/// `-O check` passes) while its TCP connection is dead — which is exactly why
/// "reconnect" otherwise does nothing until the app is restarted.
pub fn build_exit_master(
    host: &Host,
    cm: &ControlMaster,
) -> Result<Option<Vec<String>>, SshSafetyError> {
    validate(host, None)?;
    if !multiplexing_supported() {
        return Ok(None);
    }
    let path = cm.socket_dir.join("%C");
    Ok(Some(vec![
        "-o".into(),
        format!("ControlPath={}", path.display()),
        "-O".into(),
        "exit".into(),
        "--".into(),
        destination(host),
    ]))
}

/// Resolve a jump host out of a host list, for callers holding a flat store.
pub fn resolve_jump(hosts: &[Host], id: Option<Uuid>) -> Option<&Host> {
    id.and_then(|id| hosts.iter().find(|h| h.id == id))
}
