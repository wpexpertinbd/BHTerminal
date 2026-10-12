//! Where things live on each platform. Kept here, in the shell, because the
//! core deliberately never discovers paths for itself.

use std::io;
use std::path::PathBuf;

/// `~/Library/Application Support/BHTerminal` on macOS — the SAME directory
/// the shipped Swift app uses, so an existing install's sessions.json is
/// picked up as-is rather than silently starting empty.
/// `%APPDATA%\BHTerminal` on Windows, `~/.local/share/BHTerminal` on Linux.
///
/// `BHTERMINAL_DATA_DIR` overrides it. That exists for one reason: taking
/// screenshots and running tests against a throwaway set of hosts, instead of
/// pointing a camera at somebody's real server list. It is read from the
/// environment only — never from a config file or an argument — so nothing
/// the app loads can move where its credentials are kept.
pub fn data_dir() -> io::Result<PathBuf> {
    let dir = match std::env::var_os("BHTERMINAL_DATA_DIR") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => dirs::data_dir()
            .ok_or_else(|| io::Error::other("no data directory for this user"))?
            .join("BHTerminal"),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn sessions_file() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("sessions.json"))
}

pub fn vault_file() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("secrets.json"))
}

/// ⚠️ NOT under the data directory, and not arbitrary.
///
/// A Unix domain socket path is limited to ~104 bytes, and ssh expands `%C`
/// to 64 hex characters. `~/Library/Application Support/BHTerminal/cm/%C` is
/// already past the limit before the hash is even added, so multiplexing
/// would fail at runtime with a truncated-path error. A short home-relative
/// directory keeps the whole thing under ~95 bytes — which is also what the
/// Swift app uses, so both share one master during the transition.
///
/// 0700: `%C` is predictable, so a socket another user can reach is a way to
/// hijack an authenticated session.
pub fn control_socket_dir() -> io::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| io::Error::other("no home directory"))?;
    let dir = home.join(".bhterminal").join("cm");
    std::fs::create_dir_all(&dir)?;
    restrict_dir(&dir)?;
    restrict_dir(&home.join(".bhterminal"))?;
    Ok(dir)
}

fn restrict_dir(path: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
