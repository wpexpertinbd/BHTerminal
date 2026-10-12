//! The Tauri shell: paths, commands and the live-session registry.
//! Everything portable lives in `bhterm-core`; this file is the part that
//! talks to the OS and the webview.

#[allow(dead_code)]
mod paths;
mod pty;
mod sftp_session;
mod transfer;
mod tunnels;
mod updater;
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

#[tauri::command]
fn save_workspace(store: SessionStore) -> Result<(), String> {
    let path = paths::sessions_file().map_err(|e| e.to_string())?;
    store.save(&path).map_err(|e| e.to_string())
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
        },
        bhterm_core::AuthMethod::PrivateKey { .. } => AuthSecrets {
            password: None,
            passphrase: vault.get(&host.passphrase_account_for()),
        },
        bhterm_core::AuthMethod::Agent {} => AuthSecrets::default(),
    };

    let pane_id = Uuid::new_v4().to_string();
    registry.spawn(pane_id.clone(), ssh_program(), &args, cols, rows, auth, move |e| {
        on_event.send(e).is_ok()
    })?;
    Ok(pane_id)
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

    let mut store = load_store()?;
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
/// Blocking work — the handshake and the retry window both wait on the
/// network — so it runs off the UI thread.
#[tauri::command]
async fn sftp_connect(
    sftp: tauri::State<'_, SharedSftp>,
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

    let sftp = sftp.inner().clone();
    let key = host_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 45s: long enough to cover someone typing a key passphrase into the
        // terminal next door, which is what the master is waiting on.
        sftp.connect(key.clone(), ssh_program(), &args, Duration::from_secs(45))?;
        sftp.list(&key, ".")
    })
    .await
    .map_err(|e| e.to_string())?
}

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
/// open has no business holding one. The UI reports which hosts are still
/// open whenever its tabs change.
#[tauri::command]
fn sftp_prune(sftp: tauri::State<'_, SharedSftp>, keep: Vec<String>) {
    sftp.prune(&keep);
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

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(registry.clone())
        .manage(sftp.clone())
        .manage(vnc)
        .manage(vault)
        .manage(tunnels.clone())
        .invoke_handler(tauri::generate_handler![
            load_workspace,
            save_workspace,
            open_terminal,
            open_local_shell,
            write_terminal,
            resize_terminal,
            close_terminal,
            terminal_is_running,
            sftp_connect,
            sftp_list,
            sftp_mkdir,
            sftp_rename,
            sftp_delete,
            sftp_download,
            sftp_upload,
            sftp_disconnect,
            sftp_prune,
            vnc_prepare,
            save_host,
            host_has_secret,
            delete_host,
            save_folder,
            delete_folder,
            transfer::backup_export,
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
        ])
        .on_window_event(move |window, event| {
            // Every pane is a child ssh process; leaving them behind would
            // orphan connections (and their shared masters) after quit.
            if let tauri::WindowEvent::Destroyed = event {
                if window.webview_windows().len() <= 1 {
                    registry.close_all();
                    sftp.disconnect_all();
                    tunnels.stop_all();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running BHTerminal");
}
