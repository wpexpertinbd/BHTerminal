//! Where the remote working directory comes from.
//!
//! cwd-follow reads the terminal's window TITLE rather than installing a
//! shell hook. Injecting a `PROMPT_COMMAND`/`precmd` hook works, but the
//! remote shell ECHOES it — a wall of shell code in every session. Distro
//! defaults already publish the cwd in the xterm title whenever TERM matches
//! `xterm*` (which is what we set):
//!
//!   CentOS/RHEL/Alma `/etc/bashrc`  → `root@s1:~`  /  `root@s1:/home`
//!   Debian/Ubuntu `~/.bashrc`       → `root@host: ~/dir`
//!   zsh / other integrations        → `~/dir` or `/abs/path`
//!
//! A tilde is returned as-is: only the remote knows its own home, so the SFTP
//! session expands it against the directory it logged into.

/// OSC 7 (`file://host/path`) — honoured when the remote has real shell
/// integration of its own, which is more precise than the title.
pub fn parse_osc7(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        // Strip the host component; the path starts at the next slash.
        let idx = rest.find('/')?;
        let path = &rest[idx..];
        return (!path.is_empty()).then(|| path.to_string());
    }
    trimmed.starts_with('/').then(|| trimmed.to_string())
}

/// Extracts a path from a window title, or None when the title isn't one.
pub fn parse_title(title: &str) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Titles are usually "user@host:<path>" — take everything after the last
    // colon. A path that itself contains a colon fails the check below and is
    // ignored, which beats navigating somewhere wrong.
    let candidate = match trimmed.rfind(':') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
    .trim();

    // Only accept something unambiguously a path, so ordinary titles
    // ("vim foo", "htop", "user@host") are ignored.
    let looks_like_path =
        candidate == "~" || candidate.starts_with("~/") || candidate.starts_with('/');
    if !looks_like_path {
        return None;
    }
    // A hostile title must not smuggle control characters into a navigation.
    if candidate.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return None;
    }
    Some(candidate.to_string())
}

/// Expands the `~` a title reports, against the remote home the SFTP session
/// landed in. Returns None while that home is still unknown.
pub fn expand_tilde(path: &str, remote_home: Option<&str>) -> Option<String> {
    if !path.starts_with('~') {
        return Some(path.to_string());
    }
    let home = remote_home?;
    if path == "~" {
        return Some(home.to_string());
    }
    let rest = &path[2..]; // past "~/"
    Some(format!("{}/{}", home.trim_end_matches('/'), rest))
}
