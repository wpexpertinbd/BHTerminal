//! The Tauri shell: paths, commands and the live-session registry.
//! Everything portable lives in `bhterm-core`; this file is the part that
//! talks to the OS and the webview.

mod paths;
mod pty;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use bhterm_core::{ssh, SessionStore};
use pty::{PtyEvent, PtyRegistry, SharedRegistry};
use serde::Serialize;
use std::sync::Arc;
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

    let pane_id = Uuid::new_v4().to_string();
    registry.spawn(pane_id.clone(), ssh_program(), &args, cols, rows, on_event)?;
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
    registry.spawn(pane_id.clone(), &program, &args, cols, rows, on_event)?;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let registry: SharedRegistry = Arc::new(PtyRegistry::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(registry.clone())
        .invoke_handler(tauri::generate_handler![
            load_workspace,
            save_workspace,
            open_terminal,
            open_local_shell,
            write_terminal,
            resize_terminal,
            close_terminal,
            terminal_is_running,
        ])
        .on_window_event(move |window, event| {
            // Every pane is a child ssh process; leaving them behind would
            // orphan connections (and their shared masters) after quit.
            if let tauri::WindowEvent::Destroyed = event {
                if window.webview_windows().len() <= 1 {
                    registry.close_all();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running BHTerminal");
}
