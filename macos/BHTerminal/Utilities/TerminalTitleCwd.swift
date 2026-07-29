import Foundation

/// Extracts the remote working directory from the terminal's window title.
///
/// This is how cwd-follow works without injecting anything into your shell.
/// Distro defaults already publish the cwd in the xterm title whenever TERM
/// matches `xterm*` (which is what we set), so it arrives for free on every
/// prompt — no `PROMPT_COMMAND` hook typed into the session, nothing echoed:
///
///   CentOS/RHEL/Alma `/etc/bashrc`  → `root@s1:~`  /  `root@s1:/home`
///   Debian/Ubuntu `~/.bashrc`       → `root@host: ~/dir`
///   zsh/other integrations          → `~/dir` or `/abs/path`
///
/// A tilde is returned as-is; SFTPConnection expands it against the session's
/// real home directory (it can't be resolved locally).
enum TerminalTitleCwd {
    static func parsePath(fromTitle title: String) -> String? {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        // Titles are usually "user@host:<path>"; take everything after the last
        // colon. A path that itself contains a colon simply fails the check
        // below and is ignored — better than navigating somewhere wrong.
        var candidate = trimmed
        if let colon = trimmed.lastIndex(of: ":") {
            candidate = String(trimmed[trimmed.index(after: colon)...])
        }
        candidate = candidate.trimmingCharacters(in: .whitespaces)

        // Only accept something that is unambiguously a path, so ordinary
        // titles ("vim foo", "htop", "user@host") are ignored.
        guard candidate == "~" || candidate.hasPrefix("~/") || candidate.hasPrefix("/") else {
            return nil
        }
        // Guard against control characters sneaking in from a hostile title.
        guard !candidate.unicodeScalars.contains(where: { $0.value < 0x20 || $0.value == 0x7F }) else {
            return nil
        }
        return candidate
    }
}
