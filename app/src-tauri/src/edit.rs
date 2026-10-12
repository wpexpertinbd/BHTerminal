//! Editing a remote file.
//!
//! There is no magic here and that is the point: the file is fetched to a
//! private scratch directory, something edits it, and BHTerminal writes it
//! back. The built-in editor saves on demand; an external editor is watched,
//! so pressing ⌘S in another app lands on the server a moment later.
//!
//! The scratch directory is owner-only, because a copy of a server's config
//! file is as sensitive as the original.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Big enough for configs, logs and scripts; small enough that the editor
/// never becomes a way to pull a database dump into a text box.
pub const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;

fn scratch_dir() -> Result<PathBuf, String> {
    let dir = crate::paths::data_dir().map_err(|e| e.to_string())?.join("edits");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// A local path for one remote file. The host and a hash of the remote path
/// keep two servers' `/etc/hosts` apart, and the real file name is kept last
/// so an editor still shows something recognisable in its title bar.
pub fn scratch_path(host_id: &str, remote: &str) -> Result<PathBuf, String> {
    let name = remote.rsplit('/').next().unwrap_or("file");
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || "._-".contains(c) { c } else { '-' })
        .take(80)
        .collect();
    // Not a cryptographic hash — it only has to separate paths from each
    // other, and it keeps the local name short.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in remote.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let short_host: String = host_id.chars().take(8).collect();
    let dir = scratch_dir()?.join(format!("{short_host}-{hash:016x}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir.join(if safe.is_empty() { "file".to_string() } else { safe }))
}

/// Text, or a reason it should not be opened in a text box.
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("{path}: {e}"))?;
    if meta.len() > MAX_EDIT_BYTES {
        return Err(format!(
            "that file is {:.1} MB — too big for the built-in editor. Open it with another app, \
             or edit it over the terminal.",
            meta.len() as f64 / (1024.0 * 1024.0)
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    // A NUL byte is the cheap, reliable tell for "this is not text". Showing
    // a binary in a text box and writing it back would corrupt it silently.
    if bytes.contains(&0) {
        return Err("that looks like a binary file, so BHTerminal will not edit it as text".into());
    }
    String::from_utf8(bytes)
        .map_err(|_| "that file is not valid UTF-8, so editing it here could corrupt it".to_string())
}

#[tauri::command]
pub fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("{path}: {e}"))
}

/// Seconds-resolution mtime, for noticing that another app saved the file.
/// Polled rather than watched: an editor save is a human-speed event, and a
/// poll needs no file-watching dependency and no platform differences.
#[tauri::command]
pub fn file_mtime(path: String) -> Result<u64, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("{path}: {e}"))?;
    let modified = meta.modified().map_err(|e| e.to_string())?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0))
}

/// Hands the file to another application: a named one, or whatever the
/// system opens that type with.
///
/// `app` is an application NAME, not a command line — it is passed to the
/// opener as a single argument, so it cannot carry flags of its own.
#[tauri::command]
pub fn open_in_app(
    handle: tauri::AppHandle,
    path: String,
    app: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    let app = app.filter(|a| !a.is_empty());

    // On macOS `open -a <App>` is how a specific application is chosen, and
    // the plugin has no equivalent. Everywhere else, and whenever no app is
    // named, the system's own association is what opens it.
    #[cfg(target_os = "macos")]
    if let Some(app) = app.as_deref() {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("-a")
            .arg(app)
            .arg(&file)
            .status()
            .map_err(|e| e.to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("could not open that file with {app} ({status})"))
        };
    }

    let _ = &app;
    handle
        .opener()
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Where a remote file will be staged locally.
#[tauri::command]
pub fn edit_scratch_path(host_id: String, remote: String) -> Result<String, String> {
    Ok(scratch_path(&host_id, &remote)?.display().to_string())
}

/// Removes a staged copy once the editor is finished with it — a server's
/// config should not sit in a cache directory indefinitely.
#[tauri::command]
pub fn discard_scratch(path: String) -> Result<(), String> {
    let path = Path::new(&path);
    let root = scratch_dir()?;
    // Only ever inside the scratch directory, whatever the caller says.
    if !path.starts_with(&root) {
        return Err("that path is not a staged edit".into());
    }
    if path.is_file() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    if let Some(parent) = path.parent() {
        if parent != root {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(())
}
