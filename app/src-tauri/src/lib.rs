//! The Tauri shell: paths, commands and the live-session registry.
//! Everything portable lives in `bhterm-core`; this file is the part that
//! talks to the OS and the webview.

mod archive;
mod edit;
// vault_file() is part of the paths contract but only the Vault opens it.
#[allow(dead_code)]
mod paths;
mod proc;
mod pty;
mod sftp_session;
mod transfer;
mod tunnels;
mod updater;
mod askpass;
mod vault;
mod vnc_bridge;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use bhterm_core::{ssh, SessionStore};
use pty::{AuthSecrets, PtyEvent, PtyRegistry, SharedRegistry};
use sftp_session::{Listing, SftpRegistry, SharedSftp};
use tunnels::{SharedTunnels, TunnelRegistry};
use vault::{SharedVault, Vault};
use vnc_bridge::{SharedVnc, VncBridge};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::Manager;
use uuid::Uuid;

#[derive(Serialize)]
pub struct Workspace {
    store: SessionStore,
    /// Surfaced so the UI can explain, rather than quietly behave
    /// differently, when SFTP has to authenticate a second time.
    multiplexing: bool,
}

#[tauri::command]
fn load_workspace() -> Result<Workspace, String> {
    let path = paths::sessions_file().map_err(|e| e.to_string())?;
    let store = SessionStore::load(&path).map_err(|e| e.to_string())?;
    Ok(Workspace { store, multiplexing: ssh::multiplexing_supported() })
}


/// Starts `ssh` for a saved host in a fresh pty and streams it back.
#[tauri::command]
fn open_terminal(
    registry: tauri::State<'_, SharedRegistry>,
    vault: tauri::State<'_, SharedVault>,
    host_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let host_id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(host_id).ok_or("no such host")?.clone();
    let jump = ssh::resolve_jump(&store.hosts, host.jump_host_id).cloned();

    let socket_dir = paths::control_socket_dir().map_err(|e| e.to_string())?;
    let cm = ssh::ControlMaster { socket_dir: &socket_dir };
    // The allow-list runs here, before anything is spawned: a hostname or
    // username shaped like an option is refused rather than escaped.
    let args = ssh::build_terminal(&host, jump.as_ref(), Some(&cm)).map_err(|e| e.to_string())?;

    // Only the secret the host's auth method can actually use is handed over.
    let auth = match host.auth_method {
        bhterm_core::AuthMethod::Password {} => AuthSecrets {
            password: vault.get(&host.secret_account_for()),
            passphrase: None,
            notify: true,
            agent: false,
        },
        bhterm_core::AuthMethod::PrivateKey { .. } => AuthSecrets {
            password: None,
            passphrase: vault.get(&host.passphrase_account_for()),
            notify: true,
            agent: false,
        },
        bhterm_core::AuthMethod::Agent {} => AuthSecrets { notify: true, agent: true, ..AuthSecrets::default() },
    };

    let pane_id = Uuid::new_v4().to_string();
    registry.spawn(pane_id.clone(), ssh_program(), &args, cols, rows, auth, move |e| {
        on_event.send(e).is_ok()
    })?;
    Ok(pane_id)
}

/// After "REMOTE HOST IDENTIFICATION HAS CHANGED": forgets the saved key of
/// the server ssh complained about, so the next connect asks to trust the key
/// it presents now — what MobaXterm's "trust the new key?" does.
///
/// `reported` is the name from ssh's own warning. It is only used to choose
/// between this host and its jump host; anything else is refused, so text in
/// a terminal cannot make the app touch some other server's entry.
///
/// The name and the files come from `ssh -G` — ssh's own reading of the
/// user's config — so a `HostKeyAlias` or a custom `UserKnownHostsFile` is
/// honoured. Only files that actually hold the key are rewritten (by the
/// system's `ssh-keygen -R`, which keeps `<file>.old`), each is re-checked
/// afterwards, and if none held it the user is told so instead of being
/// shown a success that changed nothing.
#[tauri::command]
async fn forget_host_key(host_id: String, reported: String) -> Result<String, String> {
    let host_id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(host_id).ok_or("no such host")?.clone();
    let jump = ssh::resolve_jump(&store.hosts, host.jump_host_id).cloned();

    tauri::async_runtime::spawn_blocking(move || {
        let mut owner = None;
        for candidate in std::iter::once(&host).chain(jump.as_ref()) {
            let cfg = effective_host_key_config(candidate)?;
            if ssh::reported_is(candidate, &cfg, &reported) {
                owner = Some((candidate.clone(), cfg));
                break;
            }
        }
        let (owner, cfg) = owner.ok_or_else(|| {
            format!("“{reported}” is not this host or its jump host, so nothing was changed.")
        })?;
        let name = ssh::host_key_name(&owner, &cfg);

        let keygen = |args: Vec<String>| {
            proc::command("ssh-keygen")
                .args(&args)
                .output()
                .map_err(|e| format!("Couldn't run ssh-keygen: {e}"))
        };
        let mut cleaned = Vec::new();
        for file in cfg.files.iter().filter(|f| std::path::Path::new(f).is_file()) {
            if !keygen(ssh::build_find_host_key(&name, file))?.status.success() {
                continue; // no key for this server in that file
            }
            let out = keygen(ssh::build_forget_host_key(&name, file))?;
            if !out.status.success() {
                let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(format!("ssh-keygen couldn't remove the old key for {name} from {file}: {said}"));
            }
            // Check the effect, not the exit code.
            if keygen(ssh::build_find_host_key(&name, file))?.status.success() {
                return Err(format!("A key for {name} is still saved in {file} — remove it by hand."));
            }
            cleaned.push(file.clone());
        }
        if cleaned.is_empty() {
            return Err(format!(
                "No saved key for {name} was found in {} — nothing was changed. \
                 If your ssh config keeps keys somewhere else, remove it there.",
                cfg.files.join(", ")
            ));
        }
        Ok(format!("{name} ({})", cleaned.join(", ")))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// After "UNPROTECTED PRIVATE KEY FILE": makes the host's saved key file
/// readable by the user alone, so ssh stops ignoring it.
///
/// Only the key file saved on THIS host is touched — never a path taken from
/// terminal output — and the change only ever tightens access. A symlink or a
/// missing file is refused, a file swapped for a link mid-way is never
/// followed, and the result is read back before success is reported.
#[tauri::command]
async fn fix_key_permissions(host_id: String) -> Result<String, String> {
    let host_id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(host_id).ok_or("no such host")?.clone();
    let bhterm_core::AuthMethod::PrivateKey { path } = &host.auth_method else {
        return Err("This host doesn't log in with a key file.".into());
    };
    let path = ssh::expand_tilde(path);
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("Couldn't find the key file {path}: {e}"))?;
        if !meta.file_type().is_file() {
            return Err(format!("{path} is not a plain file, so it was left alone."));
        }
        make_private(&path, &meta)?;
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Unix: change the mode through an open handle, after checking the handle is
/// the very file that was just checked as a plain file (same device + inode).
/// A link swapped in between would open something else and be refused.
#[cfg(unix)]
fn make_private(path: &str, checked: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't open {path}: {e}"))?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if opened.dev() != checked.dev() || opened.ino() != checked.ino() {
        return Err(format!("{path} changed while it was being fixed, so it was left alone."));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Couldn't change the permissions of {path}: {e}"))?;
    let mode = file.metadata().map_err(|e| e.to_string())?.permissions().mode();
    if !bhterm_core::keyperms::unix_mode_is_private(mode) {
        return Err(format!("{path} is still readable by others (mode {:o}).", mode & 0o777));
    }
    Ok(())
}

#[cfg(windows)]
fn make_private(path: &str, _checked: &std::fs::Metadata) -> Result<(), String> {
    make_private_windows(path)
}

// Compiled on every platform (and used only on Windows) so a Mac build still
// type-checks it.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_tool(program: &str, args: Vec<String>) -> Result<std::process::Output, String> {
    proc::command(program)
        .args(&args)
        .output()
        .map_err(|e| format!("Couldn't run {program}: {e}"))
}

/// One icacls step that must work; its own message is passed on if it fails.
#[cfg_attr(not(windows), allow(dead_code))]
fn icacls_step(args: Vec<String>, what: &str) -> Result<std::process::Output, String> {
    let out = run_tool("icacls", args)?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("Windows refused to {what}: {} {}", said, err).trim().to_string());
    }
    Ok(out)
}

/// The entries on the file — a failed read is an error, never an empty list.
#[cfg_attr(not(windows), allow(dead_code))]
fn acl_principals(path: &str) -> Result<Vec<String>, String> {
    use bhterm_core::keyperms::*;
    let out = icacls_step(build_icacls_list(path), &format!("read the permissions of {path}"))?;
    Ok(parse_icacls(&String::from_utf8_lossy(&out.stdout), path))
}

/// Windows: ssh reads the ACL and the owner. Drop inherited entries, give the
/// user (by SID) full control, make them the owner, remove every entry left
/// that isn't the user, and read the ACL back to prove it.
#[cfg_attr(not(windows), allow(dead_code))]
fn make_private_windows(path: &str) -> Result<(), String> {
    use bhterm_core::keyperms::*;
    let move_hint = "Move the key into your .ssh folder and pick it again in the host's settings.";
    let who = run_tool("whoami", vec!["/user".into(), "/fo".into(), "csv".into(), "/nh".into()])?;
    let (me, sid) = parse_whoami_user(&String::from_utf8_lossy(&who.stdout))
        .ok_or("Couldn't tell which Windows account is running BHTerminal.")?;
    if !name_is_trustworthy(&me) {
        return Err(format!("Couldn't read your account name reliably, so nothing was changed. {move_hint}"));
    }

    icacls_step(build_icacls_restrict(path, &sid), &format!("change the permissions of {path}"))?;
    // ssh also rejects a key owned by another ordinary account. With full
    // control just granted, taking ownership needs no special privilege.
    icacls_step(build_icacls_owner(path, &sid), &format!("make you the owner of {path}"))
        .map_err(|e| format!("{e}. {move_hint}"))?;

    let found = acl_principals(path)?;
    if let Some(bad) = found.iter().find(|p| !name_is_trustworthy(p)) {
        return Err(format!("Couldn't read an account name on {path} reliably ({bad}). {move_hint}"));
    }
    for other in principals_to_remove(&found, &me) {
        icacls_step(build_icacls_remove(path, &other), &format!("remove {other} from {path}"))?;
    }
    acl_is_private(&acl_principals(path)?, &me).map_err(|why| format!("{path}: {why}. {move_hint}"))
}

/// ssh's own answer to "which name and files hold this server's key". If
/// `ssh -G` can't run, fall back to ssh's defaults.
fn effective_host_key_config(host: &bhterm_core::Host) -> Result<ssh::HostKeyConfig, String> {
    let args = ssh::build_effective_config(host).map_err(|e| e.to_string())?;
    let mut cfg = proc::command(ssh_program())
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| ssh::parse_effective_config(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    if cfg.files.is_empty() {
        cfg.files = vec![ssh::expand_tilde("~/.ssh/known_hosts")];
    }
    cfg.files = cfg.files.iter().map(|f| ssh::expand_tilde(f)).collect();
    Ok(cfg)
}

/// A local shell, for trying the terminal without a server.
#[tauri::command]
fn open_local_shell(
    registry: tauri::State<'_, SharedRegistry>,
    cols: u16,
    rows: u16,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let pane_id = Uuid::new_v4().to_string();
    let (program, args) = default_shell();
    registry.spawn(pane_id.clone(), &program, &args, cols, rows, AuthSecrets::default(), move |e| {
        on_event.send(e).is_ok()
    })?;
    Ok(pane_id)
}

#[tauri::command]
fn write_terminal(
    registry: tauri::State<'_, SharedRegistry>,
    pane_id: String,
    data: String,
) -> Result<(), String> {
    let bytes = B64.decode(&data).map_err(|_| "bad input encoding".to_string())?;
    registry.write(&pane_id, &bytes)
}

#[tauri::command]
fn resize_terminal(
    registry: tauri::State<'_, SharedRegistry>,
    pane_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    registry.resize(&pane_id, cols, rows)
}

#[tauri::command]
fn close_terminal(registry: tauri::State<'_, SharedRegistry>, pane_id: String) {
    registry.close(&pane_id);
}

#[tauri::command]
fn terminal_is_running(registry: tauri::State<'_, SharedRegistry>, pane_id: String) -> bool {
    registry.is_running(&pane_id)
}

/// Windows 10+ ships OpenSSH, so the same binary name works everywhere and
/// we keep ~/.ssh/config, the agent, ProxyJump, 2FA and host-key TOFU for
/// free rather than reimplementing any of it.
fn ssh_program() -> &'static str {
    "ssh"
}

fn default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        ("powershell.exe".to_string(), vec!["-NoLogo".to_string()])
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        // Login shell, so the user's own prompt and PATH apply.
        (shell, vec!["-l".to_string()])
    }
}



// ── hosts, folders and their secrets ───────────────────────────────────────

fn load_store() -> Result<SessionStore, String> {
    SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn save_store(store: &SessionStore) -> Result<(), String> {
    store.save(&paths::sessions_file().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Rewrites a key path inside the user's own home as `~/…`.
///
/// ⚠️ This is what makes a host list portable. A Mac records
/// `/Users/benjamin/.ssh/id_ed25519`; carried to Windows that path does not
/// exist and the connection quietly falls back to asking for a password.
/// `~/.ssh/id_ed25519` resolves on BOTH (ssh.rs expands `~` from HOME or
/// USERPROFILE), so the same entry works on either machine. Backslashes are
/// normalised because ssh accepts forward slashes everywhere.
fn portable_key_path(path: &str) -> String {
    let normalised = path.replace('\\', "/");
    let Some(home) = dirs::home_dir() else { return normalised };
    let home = home.to_string_lossy().replace('\\', "/");
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return normalised;
    }
    // Windows paths are case-insensitive; comparing lowercased is right there
    // and harmless elsewhere, since a real prefix still matches.
    let (hay, needle) = if cfg!(windows) {
        (normalised.to_lowercase(), home.to_lowercase())
    } else {
        (normalised.clone(), home.to_string())
    };
    match hay.strip_prefix(&needle).and_then(|rest| rest.strip_prefix('/')) {
        Some(_) => format!("~/{}", &normalised[home.len() + 1..]),
        None => normalised,
    }
}

/// Whether a key file is actually on this computer, for the editor to warn
/// about a path that came from another machine.
#[tauri::command]
fn key_path_exists(path: String) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    std::path::Path::new(&ssh::expand_tilde(&path)).is_file()
}

/// Adds or updates a host, and stores (or clears) its secret in one step.
///
/// An EMPTY secret means "remove it", not "leave it alone". Saving only
/// non-empty values is how the Swift build ended up unable to clear a
/// passphrase: the field looked cleared and the old value quietly lived on.
#[tauri::command]
fn save_host(
    vault: tauri::State<'_, SharedVault>,
    host: bhterm_core::Host,
    secret: Option<String>,
) -> Result<bhterm_core::Host, String> {
    // Refuse a value the argv builder would reject anyway, so the error lands
    // on the field that caused it instead of on a failed connection later.
    // (`ssh -- ` is not available for these, so a leading "-" really is an
    // option-injection vector; the builder is still the enforcement point.)
    if !ssh::is_valid_hostname(&host.hostname) {
        return Err(format!("“{}” is not a usable hostname or address.", host.hostname));
    }
    if host.connection_type == bhterm_core::ConnectionType::Ssh
        && !ssh::is_valid_username(&host.username)
    {
        return Err(format!("“{}” is not a usable username.", host.username));
    }

    // Store a key path portably before anything else sees it.
    let mut host = host;
    if let bhterm_core::AuthMethod::PrivateKey { path } = &host.auth_method {
        host.auth_method = bhterm_core::AuthMethod::PrivateKey {
            path: portable_key_path(path),
        };
    }

    let mut store = load_store()?;
    // A saved secret belongs to the server it was saved for. If the address,
    // port, user or jump host changes and no new secret is typed, the old one
    // is forgotten rather than carried over — otherwise editing a host to
    // point at a server someone else controls, then connecting, would type
    // the real password into THEIR server, master password or not.
    let retargeted = store.host(host.id).is_some_and(|old| !old.same_destination(&host));
    let saved = if store.host(host.id).is_some() {
        store.update_host(host.clone());
        host
    } else {
        store.add_host(host)
    };
    save_store(&store)?;

    // Route the secret to the account its auth method actually uses, and
    // clear the one it doesn't, so switching method leaves nothing stale.
    let (used, unused) = match saved.auth_method {
        bhterm_core::AuthMethod::Password {} => {
            (Some(saved.secret_account_for()), saved.passphrase_account_for())
        }
        bhterm_core::AuthMethod::PrivateKey { .. } => {
            (Some(saved.passphrase_account_for()), saved.secret_account_for())
        }
        bhterm_core::AuthMethod::Agent {} => (None, saved.secret_account_for()),
    };
    vault.remove(&[unused])?;
    if let Some(account) = used {
        match secret.as_deref() {
            Some(value) if !value.is_empty() => vault.set(&account, value)?,
            Some(_) => vault.remove(&[account])?,
            None if retargeted => vault.remove(&[account])?,
            None => {} // not supplied: leave whatever is stored untouched
        }
    } else {
        vault.remove(&[saved.passphrase_account_for()])?;
    }
    Ok(saved)
}

/// Whether a secret is stored, so the editor can show "saved" without ever
/// sending the secret itself back to the webview.
#[tauri::command]
fn host_has_secret(vault: tauri::State<'_, SharedVault>, host_id: String) -> bool {
    let Ok(id) = host_id.parse::<Uuid>() else { return false };
    vault.get(&bhterm_core::Host::secret_account(id)).is_some()
        || vault.get(&bhterm_core::Host::passphrase_account(id)).is_some()
}

#[tauri::command]
fn delete_host(
    vault: tauri::State<'_, SharedVault>,
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
) -> Result<(), String> {
    let id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let mut store = load_store()?;
    let freed = store.delete_host(id);
    save_store(&store)?;
    vault.remove(&freed)?;
    sftp.disconnect(&host_id);
    Ok(())
}

#[tauri::command]
fn save_folder(folder: bhterm_core::HostFolder) -> Result<bhterm_core::HostFolder, String> {
    let mut store = load_store()?;
    let saved = if store.folders.iter().any(|f| f.id == folder.id) {
        store.update_folder(folder.clone());
        folder
    } else {
        store.add_folder(folder)
    };
    save_store(&store)?;
    Ok(saved)
}

/// Deletes a folder and everything inside it, freeing those hosts' secrets.
#[tauri::command]
fn delete_folder(
    vault: tauri::State<'_, SharedVault>,
    folder_id: String,
) -> Result<(), String> {
    let id: Uuid = folder_id.parse().map_err(|_| "bad folder id".to_string())?;
    let mut store = load_store()?;
    let freed = store.delete_folder(id);
    save_store(&store)?;
    vault.remove(&freed)
}


// ── session logging ────────────────────────────────────────────────────────

/// Starts a transcript for one pane. The path comes from the caller (the user
/// picked it in a save sheet), so the only default here is the suggested
/// FILENAME, built from the host and the date.
#[tauri::command]
fn start_logging(
    registry: tauri::State<'_, SharedRegistry>,
    pane_id: String,
    path: String,
) -> Result<(), String> {
    registry.start_logging(&pane_id, std::path::Path::new(&path))
}

#[tauri::command]
fn stop_logging(registry: tauri::State<'_, SharedRegistry>, pane_id: String) {
    registry.stop_logging(&pane_id);
}

#[tauri::command]
fn logging_path(registry: tauri::State<'_, SharedRegistry>, pane_id: String) -> Option<String> {
    registry.log_path(&pane_id)
}

/// A filename nobody has to invent: the host, the date and the time, in a
/// shape that sorts. Only the name — where it goes is the user's choice.
#[tauri::command]
fn suggested_log_name(host_id: Option<String>) -> String {
    let stamp = transfer::now_stamp();
    let name = host_id
        .and_then(|id| id.parse::<Uuid>().ok())
        .and_then(|id| load_store().ok().and_then(|s| s.host(id).map(|h| h.name.clone())))
        .unwrap_or_else(|| "local".into());
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    format!("{safe}-{stamp}.log")
}

// ── snippets ───────────────────────────────────────────────────────────────

#[tauri::command]
fn save_snippet(snippet: bhterm_core::Snippet) -> Result<bhterm_core::Snippet, String> {
    let mut store = load_store()?;
    let saved = if store.snippets.iter().any(|s| s.id == snippet.id) {
        store.update_snippet(snippet.clone());
        snippet
    } else {
        store.add_snippet(snippet)
    };
    save_store(&store)?;
    Ok(saved)
}

#[tauri::command]
fn delete_snippet(snippet_id: String) -> Result<(), String> {
    let id: Uuid = snippet_id.parse().map_err(|_| "bad snippet id".to_string())?;
    let mut store = load_store()?;
    store.delete_snippet(id);
    save_store(&store)
}

// ── port forwards ──────────────────────────────────────────────────────────

/// Starts one of a host's own tunnel rules. The rule is read from the store
/// rather than taken from the caller, so the listen and destination ports are
/// the ones that were saved and reviewed.
#[tauri::command]
fn tunnel_start(
    tunnels: tauri::State<'_, SharedTunnels>,
    host_id: String,
    rule_id: String,
) -> Result<(), String> {
    let host_id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let rule_id: Uuid = rule_id.parse().map_err(|_| "bad rule id".to_string())?;
    let store = load_store()?;
    let host = store.host(host_id).ok_or("no such host")?.clone();
    let rule = host
        .tunnels
        .iter()
        .find(|t| t.id == rule_id)
        .ok_or("no such tunnel on that host")?
        .clone();
    let jump = ssh::resolve_jump(&store.hosts, host.jump_host_id).cloned();

    let socket_dir = paths::control_socket_dir().map_err(|e| e.to_string())?;
    tunnels.start(&host, jump.as_ref(), &rule, Some(&socket_dir))
}

#[tauri::command]
fn tunnel_stop(tunnels: tauri::State<'_, SharedTunnels>, rule_id: String) {
    tunnels.stop(&rule_id);
}

#[tauri::command]
fn tunnel_status(
    tunnels: tauri::State<'_, SharedTunnels>,
    rule_ids: Vec<String>,
) -> Vec<tunnels::TunnelStatus> {
    tunnels.status(&rule_ids)
}

// ── SFTP ───────────────────────────────────────────────────────────────────

/// Builds the SFTP argv for a host and opens (or reuses) its session.
///
/// Opens the file pane's channel for a host.
///
/// How depends on how the host logs in, because every login attempt is one
/// the SERVER counts:
///
/// - **Password host, shared connection available (macOS/Linux):** never dial
///   it. The channel runs without a tty, so any attempt is a guaranteed failed
///   login. Wait — on the local control socket, free for the server — until
///   the terminal's own login finishes, then ride that connection.
/// - **Key / agent host:** one direct attempt (instant when the key just
///   works); if that fails the key probably needs a passphrase typed in the
///   terminal, so fall back to waiting for the terminal exactly as above.
/// - **Windows (no shared connection):** a password host logs in by itself,
///   ONCE, with the saved password handed over through askpass; with no saved
///   password there is nothing to log in with, and it says so.
///
/// Blocking work — it waits on the network and on a person — so it runs off
/// the UI thread.
#[tauri::command]
async fn sftp_connect(
    sftp: tauri::State<'_, SharedSftp>,
    vault: tauri::State<'_, SharedVault>,
    host_id: String,
) -> Result<Listing, String> {
    let id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(id).ok_or("no such host")?.clone();
    let jump = ssh::resolve_jump(&store.hosts, host.jump_host_id).cloned();
    let socket_dir = paths::control_socket_dir().map_err(|e| e.to_string())?;
    let cm = ssh::ControlMaster { socket_dir: &socket_dir };
    let args = ssh::build_sftp(&host, jump.as_ref(), Some(&cm)).map_err(|e| e.to_string())?;
    let check = ssh::build_check_master(&host, jump.as_ref(), &cm).map_err(|e| e.to_string())?;
    let is_password = matches!(host.auth_method, bhterm_core::AuthMethod::Password {});

    // Windows + password: its own login, with the saved password.
    let own_login = if check.is_none() && is_password {
        let secret = vault.get(&host.secret_account_for()).ok_or(
            "On Windows the file pane signs in on its own, so it needs a saved password. \
             Save it under Edit host (the terminal's login can't be shared here).",
        )?;
        // Through a jump host the file pane would have to answer the jump
        // host's login too, and the saved password belongs to the TARGET only.
        if jump.is_some() {
            return Err(
                "On Windows the file pane can't sign in through a jump host by itself \
                 (the saved password is for the target, not the jump host)."
                    .into(),
            );
        }
        let login_args = ssh::build_sftp_password(&host, None).map_err(|e| e.to_string())?;
        Some((login_args, askpass::env_for(&secret, &ssh::destination(&host))?))
    } else {
        None
    };

    let sftp = sftp.inner().clone();
    let key = host_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let program = ssh_program();
        match (&check, own_login) {
            // Windows, password: one attempt.
            (None, Some((login_args, env))) => {
                sftp.connect(key.clone(), program, &login_args, &env, Duration::ZERO)?;
            }
            // Windows, key or agent: no password to prompt for, retrying is cheap.
            (None, None) => {
                sftp.connect(key.clone(), program, &args, &[], Duration::from_secs(45))?;
            }
            // Shared connection, password host: wait for the terminal's login,
            // then ONE attempt over it. Not a retry loop: if the shared
            // connection vanished in between, every retry would be a failed
            // login against the server — the very thing this avoids.
            (Some(check), _) if is_password => {
                sftp.wait_for_master(&key, program, check, MASTER_WAIT)?;
                sftp.connect(key.clone(), program, &args, &[], Duration::ZERO)?;
            }
            // Shared connection, key or agent: try directly, else wait.
            (Some(check), _) => {
                if let Err(first) = sftp.connect(key.clone(), program, &args, &[], Duration::ZERO) {
                    // A network problem will not fix itself while we wait for a
                    // terminal login: say so now, with ssh's own words, instead
                    // of showing "Connecting…" for 15 minutes.
                    if is_unreachable(&first) {
                        return Err(first);
                    }
                    sftp.wait_for_master(&key, program, check, MASTER_WAIT)?;
                    sftp.connect(key.clone(), program, &args, &[], Duration::from_secs(20))?;
                }
            }
        }
        sftp.list(&key, ".")
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether an ssh error means the server cannot be reached or trusted at all —
/// as opposed to "a login is still pending", which waiting can fix.
fn is_unreachable(error: &str) -> bool {
    let e = error.to_lowercase();
    [
        "could not resolve", "connection refused", "timed out", "no route to host",
        "network is unreachable", "host is down", "host key verification failed",
        "remote host identification has changed",
    ]
    .iter()
    .any(|needle| e.contains(needle))
}

/// How long the file pane waits for someone to finish logging in in the
/// terminal. Generous: the cost of waiting is a local socket poll once a
/// second, and the alternative was giving up on a person still typing.
const MASTER_WAIT: Duration = Duration::from_secs(15 * 60);

#[tauri::command]
async fn sftp_list(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    path: String,
) -> Result<Listing, String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sftp.list(&host_id, &path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sftp_mkdir(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    path: String,
    name: String,
) -> Result<(), String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sftp.mkdir(&host_id, &sftp_session::join(&path, &name)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sftp_rename(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    from: String,
    to: String,
) -> Result<(), String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sftp.rename(&host_id, &from, &to))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sftp_delete(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sftp.delete(&host_id, &path, is_dir))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sftp_download(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    remote: String,
    local: String,
) -> Result<u64, String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sftp.download(&host_id, &remote, std::path::Path::new(&local))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sftp_upload(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    local: String,
    remote_dir: String,
) -> Result<u64, String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = std::path::Path::new(&local);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("that file has no name")?
            .to_string();
        sftp.upload(&host_id, path, &sftp_session::join(&remote_dir, &name))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// SFTP rides the terminal's connection, so a host with no terminal tab left
#[tauri::command]
async fn sftp_create_file(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    path: String,
    name: String,
) -> Result<(), String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sftp.create_file(&host_id, &sftp_session::join(&path, &name))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Uploads to an EXACT remote path. `sftp_upload` puts a file into a folder;
/// saving an edit has to land back on the file it came from, name and all.
#[tauri::command]
async fn sftp_upload_to(
    sftp: tauri::State<'_, SharedSftp>,
    host_id: String,
    local: String,
    remote: String,
) -> Result<u64, String> {
    let sftp = sftp.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sftp.upload(&host_id, std::path::Path::new(&local), &remote)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// open has no business holding one. The UI reports which hosts are still
/// open whenever its tabs change.
#[tauri::command]
fn sftp_prune(sftp: tauri::State<'_, SharedSftp>, keep: Vec<String>) {
    sftp.prune(&keep);
}

/// The file pane went away: stop waiting for a terminal login on its behalf.
#[tauri::command]
fn sftp_cancel_wait(sftp: tauri::State<'_, SharedSftp>, host_id: String) {
    sftp.cancel_wait(&host_id);
}

#[tauri::command]
fn sftp_disconnect(sftp: tauri::State<'_, SharedSftp>, host_id: String) {
    sftp.disconnect(&host_id);
}

// ── VNC ────────────────────────────────────────────────────────────────────

/// Returns a one-shot ws:// URL for this host's VNC server.
///
/// The webview never names a target: it presents a token that was registered
/// here against one specific host:port, so the bridge cannot be talked into
/// connecting anywhere else.
#[tauri::command]
async fn vnc_prepare(
    vnc: tauri::State<'_, SharedVnc>,
    host_id: String,
) -> Result<String, String> {
    let id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(id).ok_or("no such host")?.clone();
    // The same allow-list the ssh argv uses: refuse a weird hostname rather
    // than hand it to a socket.
    if !ssh::is_valid_hostname(&host.hostname) {
        return Err(format!("“{}” is not a valid hostname", host.hostname));
    }
    vnc.prepare(host.hostname.clone(), host.port).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ssh started this executable to ask for a saved password (Windows file
    // pane): answer and exit before anything of the app exists.
    askpass::run_if_helper();
    let registry: SharedRegistry = Arc::new(PtyRegistry::default());
    let sftp: SharedSftp = Arc::new(SftpRegistry::default());
    let vnc: SharedVnc = Arc::new(VncBridge::default());
    let tunnels: SharedTunnels = Arc::new(TunnelRegistry::default());
    // An unreadable vault must stop the app rather than silently present an
    // empty one — the next save would overwrite every stored credential.
    let data_dir = paths::data_dir().expect("cannot create the application data directory");
    let vault: SharedVault = Arc::new(
        Vault::open(&data_dir).expect("cannot open the credential vault"),
    );

    let mut builder = tauri::Builder::default();
    // One BHTerminal per user. Closing the window only HIDES the app (the
    // sessions live on in the tray), so launching the shortcut again used to
    // start a second copy — and a second tray icon. Now the second launch
    // hands over to the one that is running, which shows its window, and
    // exits. It must be the FIRST plugin registered.
    //
    // Skipped when BHTERMINAL_DATA_DIR is set: that is a deliberate,
    // separate instance (a test or a screenshot on throwaway data) and must
    // be able to run beside the real one rather than being swallowed by it.
    let separate_instance = std::env::var_os("BHTERMINAL_DATA_DIR").is_some_and(|v| !v.is_empty());
    if !separate_instance {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }));
    }
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(registry.clone())
        .manage(sftp.clone())
        .manage(vnc)
        .manage(vault)
        .manage(tunnels.clone())
        .invoke_handler(tauri::generate_handler![
            load_workspace,
            open_terminal,
            forget_host_key,
            fix_key_permissions,
            open_local_shell,
            write_terminal,
            resize_terminal,
            close_terminal,
            terminal_is_running,
            sftp_connect,
            sftp_list,
            sftp_mkdir,
            sftp_create_file,
            sftp_rename,
            sftp_delete,
            sftp_download,
            sftp_upload,
            sftp_upload_to,
            sftp_disconnect,
            sftp_cancel_wait,
            sftp_prune,
            vnc_prepare,
            save_host,
            host_has_secret,
            key_path_exists,
            delete_host,
            save_folder,
            delete_folder,
            transfer::backup_export,
            vault::vault_status,
            vault::vault_unlock,
            vault::vault_set_master,
            vault::vault_remove_master,
            vault::vault_set_remember,
            vault::vault_skip_setup,
            vault::vault_reset_forgotten,
            transfer::backup_is_encrypted,
            transfer::backup_restore,
            transfer::import_preview,
            transfer::import_apply,
            updater::update_query,
            updater::update_evaluate,
            updater::update_open,
            save_snippet,
            delete_snippet,
            tunnel_start,
            tunnel_stop,
            tunnel_status,
            start_logging,
            stop_logging,
            logging_path,
            suggested_log_name,
            set_window_theme,
            archive::sftp_compress,
            archive::sftp_extract,
            edit::read_text_file,
            edit::write_text_file,
            edit::file_mtime,
            edit::open_in_app,
            edit::edit_scratch_path,
            edit::discard_scratch,
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window HIDES it. The sessions, the forwards and the
            // SFTP connections live in Rust and keep running — closing the
            // window is "put it away", not "drop my logins". Quitting is an
            // explicit act, from the tray or the app menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while starting BHTerminal")
        // `_app` is read only by the macOS reopen arm below; the underscore
        // keeps the other platforms warning-free without hiding the binding.
        .run(move |_app, event| {
            match event {
            // Clicking the Dock icon with no window on screen brings the one
            // that was hidden back, rather than doing nothing.
            // ⚠️ macOS only — the variant does not exist elsewhere, and an
            // ungated arm fails to compile on Windows and Linux.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main_window(_app),
            // The ONE place everything is torn down. Every pane is a child
            // ssh process and every forward is another; leaving them behind
            // would orphan connections and their shared masters.
            tauri::RunEvent::Exit => {
                registry.close_all();
                sftp.disconnect_all();
                tunnels.stop_all();
            }
            _ => {}
            }
        });
}

/// Brings the main window back and focuses it, unhiding the app first — on
/// macOS a hidden app's window stays invisible however many times it is told
/// to show itself.
fn show_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The menu-bar glyph, embedded so it resolves inside the bundle on every
/// platform. Drawn as black-plus-alpha; see the comment at its use.
const MENUBAR_ICON: &[u8] = include_bytes!("../icons/menubar-template.png");

/// The menu-bar item: what keeps BHTerminal reachable once its window is
/// closed, and the only place that really quits it.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open BHTerminal", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit BHTerminal", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    // A dedicated monochrome glyph, NOT the app icon: the app icon is a
    // filled rounded square, so as a template it renders as a featureless
    // blob. As a template image macOS recolours this to match the menu bar —
    // white on a dark one, black on a light one — so it sits with the other
    // status items instead of shouting over them.
    let tray = TrayIconBuilder::with_id("main")
        .icon(tauri::image::Image::from_bytes(MENUBAR_ICON)?)
        .icon_as_template(true)
        .tooltip("BHTerminal")
        .menu(&menu)
        // The menu belongs on a right-click; a left-click should just bring
        // the window back, which is what people expect of a menu-bar app.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            // exit() runs RunEvent::Exit, which is where the ssh children,
            // the forwards and the SFTP sessions are closed.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    // ⚠️ The TrayIcon has to be KEPT. Dropping the value returned by build()
    // takes the item straight back out of the menu bar — which looks exactly
    // like the tray never being created at all.
    app.manage(tray);
    Ok(())
}

/// Matches the native window chrome to the appearance the user picked.
///
/// The CSS `color-scheme` handles everything inside the webview, but the
/// titlebar is the OS's, and it keeps whatever the system says — which is how
/// a light window ends up with a dark titlebar, and black title text on it.
/// `None` hands the decision back to the system.
#[tauri::command]
fn set_window_theme(window: tauri::Window, theme: Option<String>) -> Result<(), String> {
    let theme = match theme.as_deref() {
        Some("dark") => Some(tauri::Theme::Dark),
        Some("light") => Some(tauri::Theme::Light),
        // Anything else, "system" included, follows the OS again.
        _ => None,
    };
    window.set_theme(theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod connect_tests {
    use super::is_unreachable;

    #[test]
    fn network_failures_are_reported_at_once_and_login_failures_are_waited_out() {
        for fatal in [
            "ssh: connect to host 10.0.0.1 port 22: Connection refused",
            "ssh: connect to host x port 22: Operation timed out",
            "ssh: Could not resolve hostname nope.example: nodename nor servname provided",
            "ssh: connect to host x port 22: No route to host",
            "Host key verification failed.",
            "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
        ] {
            assert!(is_unreachable(fatal), "should fail fast: {fatal}");
        }
        for pending in [
            "failed to fill whole buffer — ssh said: root@h: Permission denied (publickey,password).",
            "failed to fill whole buffer — ssh said: Enter passphrase for key: ",
            "failed to fill whole buffer",
            "Connection closed by 203.0.113.9 port 22",
        ] {
            assert!(!is_unreachable(pending), "should keep waiting: {pending}");
        }
    }
}

#[cfg(all(test, unix))]
mod key_permission_tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_world_readable_key_is_made_private() {
        let dir = std::env::temp_dir().join(format!("bhterm-keyperm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id_test");
        std::fs::write(&key, b"not a real key").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();

        let meta = std::fs::symlink_metadata(&key).unwrap();
        super::make_private(key.to_str().unwrap(), &meta).unwrap();
        let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The file checked as plain must be the one changed: anything swapped in
    /// at that path after the check (a link to another file) is refused.
    #[test]
    fn a_file_swapped_after_the_check_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("bhterm-keyswap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id_test");
        let other = dir.join("someone_elses_file");
        std::fs::write(&key, b"key").unwrap();
        std::fs::write(&other, b"other").unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o644)).unwrap();
        let checked = std::fs::symlink_metadata(&key).unwrap();
        // The swap: the key's name now points at the other file.
        std::fs::remove_file(&key).unwrap();
        std::os::unix::fs::symlink(&other, &key).unwrap();

        assert!(super::make_private(key.to_str().unwrap(), &checked).is_err());
        let mode = std::fs::metadata(&other).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "the other file must be untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
