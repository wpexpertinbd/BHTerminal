import Foundation

/// Bridges the SFTP browser's "Follow Terminal" checkbox to the terminal
/// sessions.
///
/// The cwd-reporting shell hook (the bash/zsh `PROMPT_COMMAND` snippet that
/// emits OSC 7) has to be *typed into the shell* to install it, so the shell
/// echoes it once — a wall of text right after login. There's no point paying
/// that cost when the user isn't actually following the terminal, so the hook
/// is now injected **only while "Follow Terminal" is on** (off by default).
/// SFTP still auto-connects reliably via a separate, silent ready-signal
/// (`PTYSession.onReady`) that needs no shell hook.
///
/// One shared toggle is enough — there's a single SFTP pane. Main-thread only
/// by construction (SwiftUI views + the @MainActor SFTPConnection).
final class CwdFollowCenter {
    static let shared = CwdFollowCenter()
    private(set) var isEnabled = false

    /// Posted when following flips ON so already-connected terminals inject the
    /// hook at that moment, not only freshly-opened ones.
    static let didEnable = Notification.Name("com.biswashost.bhterminal.cwdFollow.didEnable")

    func setEnabled(_ on: Bool) {
        guard on != isEnabled else { return }
        isEnabled = on
        if on { NotificationCenter.default.post(name: Self.didEnable, object: nil) }
    }
}
