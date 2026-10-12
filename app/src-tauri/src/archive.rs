//! Compressing and extracting on the remote host.
//!
//! SFTP has no "zip this" operation, so these run `tar`/`zip` over ssh on the
//! other end — which keeps a 2 GB folder on the server instead of pulling it
//! down and pushing it back.
//!
//! ⚠️ The webview is deliberately NOT given a "run this command" door. It
//! names a directory and some entries in it; the command itself is built
//! here, from a fixed template, with every value shell-quoted. Entry names
//! are also checked to be plain names in that one directory, so a listing
//! from a hostile server cannot reach outside it.

use bhterm_core::ssh::{self, shell_quote};
use bhterm_core::SessionStore;
use uuid::Uuid;

use crate::paths;

fn load_host(host_id: &str) -> Result<(bhterm_core::Host, Option<bhterm_core::Host>), String> {
    let id: Uuid = host_id.parse().map_err(|_| "bad host id".to_string())?;
    let store = SessionStore::load(&paths::sessions_file().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let host = store.host(id).ok_or("no such host")?.clone();
    let jump = ssh::resolve_jump(&store.hosts, host.jump_host_id).cloned();
    Ok((host, jump))
}

/// A directory has to be absolute, and an entry has to be a plain name in it:
/// no separators, and not `.` or `..`.
fn check_dir(dir: &str) -> Result<(), String> {
    if !dir.starts_with('/') {
        return Err(format!("“{dir}” is not an absolute path"));
    }
    Ok(())
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(format!("“{name}” is not a name in this folder"));
    }
    Ok(())
}

fn run(host_id: &str, command: &str) -> Result<String, String> {
    let (host, jump) = load_host(host_id)?;
    let socket_dir = paths::control_socket_dir().map_err(|e| e.to_string())?;
    let cm = ssh::ControlMaster { socket_dir: &socket_dir };
    let args = ssh::build_exec(&host, jump.as_ref(), Some(&cm), command).map_err(|e| e.to_string())?;

    let out = crate::proc::command("ssh")
        .args(&args)
        .output()
        .map_err(|e| format!("could not run ssh: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    // The remote tool's own message is the only thing that explains a
    // failure — a missing `zip`, a read-only directory, a full disk.
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("the command failed ({})", out.status)
    } else {
        stderr
    })
}

/// Packs `names` (entries of `dir`) into one archive in the same directory,
/// and returns the archive's name. One entry keeps its own name; several get
/// a dated archive, because guessing a collective name for them would be
/// worse than saying when it was made.
#[tauri::command]
pub async fn sftp_compress(
    host_id: String,
    dir: String,
    names: Vec<String>,
    format: String,
) -> Result<String, String> {
    check_dir(&dir)?;
    if names.is_empty() {
        return Err("nothing selected".into());
    }
    for name in &names {
        check_name(name)?;
    }

    let stem = if names.len() == 1 {
        names[0].clone()
    } else {
        format!("archive-{}", crate::transfer::now_stamp())
    };

    // ⚠️ `zip` is NOT installed on a stock Ubuntu or Debian server, while
    // `tar` always is. Saying so plainly beats "exit status 127" — this was
    // the whole of "zip does nothing" on a real server.
    let (archive, tool, requires) = match format.as_str() {
        // -q so a thousand file names do not come back as "output".
        "zip" => (format!("{stem}.zip"), "zip -r -q", Some("zip")),
        "tar.gz" => (format!("{stem}.tar.gz"), "tar -czf", None),
        other => return Err(format!("unknown archive format “{other}”")),
    };

    let precheck = requires.map_or(String::new(), |tool| {
        format!(
            "command -v {tool} >/dev/null 2>&1 || {{ echo '{tool} is not installed on this server              — install it (apt install {tool}) or use .tar.gz instead' >&2; exit 127; }}; "
        )
    });

    let quoted: Vec<String> = names.iter().map(|n| shell_quote(n)).collect();
    // `set -e` so a failure to create the archive is not followed by a
    // cheerful exit code from the last command in the chain.
    let command = format!(
        "set -e; cd {}; {}test ! -e {} || {{ echo 'that archive already exists' >&2; exit 1; }}; {} {} {}",
        shell_quote(&dir),
        precheck,
        shell_quote(&archive),
        tool,
        shell_quote(&archive),
        quoted.join(" "),
    );
    let host_id = host_id.clone();
    tauri::async_runtime::spawn_blocking(move || run(&host_id, &command))
        .await
        .map_err(|e| e.to_string())??;
    Ok(archive)
}

/// Unpacks one archive into its own directory, choosing the tool from the
/// name — the server may not have `unzip`, and it says so itself if not.
#[tauri::command]
pub async fn sftp_extract(host_id: String, dir: String, name: String) -> Result<(), String> {
    check_dir(&dir)?;
    check_name(&name)?;

    let lower = name.to_ascii_lowercase();
    let tool = if lower.ends_with(".zip") {
        "unzip -o -q"
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "tar -xzf"
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz") || lower.ends_with(".tbz2") {
        "tar -xjf"
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        "tar -xJf"
    } else if lower.ends_with(".tar") {
        "tar -xf"
    } else if lower.ends_with(".gz") {
        "gunzip -kf"
    } else {
        return Err(format!("“{name}” is not an archive BHTerminal can extract"));
    };

    let needed = tool.split_whitespace().next().unwrap_or("tar");
    let command = format!(
        "set -e; cd {}; command -v {needed} >/dev/null 2>&1 || {{ echo '{needed} is not installed          on this server (apt install {needed})' >&2; exit 127; }}; {} {}",
        shell_quote(&dir),
        tool,
        shell_quote(&name)
    );
    let host_id = host_id.clone();
    tauri::async_runtime::spawn_blocking(move || run(&host_id, &command))
        .await
        .map_err(|e| e.to_string())??;
    Ok(())
}
