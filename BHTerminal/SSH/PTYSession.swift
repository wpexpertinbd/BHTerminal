import AppKit
import SwiftTerm

/// Subclasses SwiftTerm's `LocalProcessTerminalView` — its own docs call
/// this out as the sanctioned extension point ("If you want additional
/// control ... you can subclass this and override the methods").
///
/// Two things are added here, both by watching the raw output stream before
/// it's fed to the terminal — everything else is unmodified real `ssh` in a
/// pty, no protocol reimplementation:
///  1. Auto-filling a Keychain-stored password at ssh's interactive
///     password prompt.
///  2. Injecting a one-line bash/zsh prompt hook, once output has settled
///     (i.e. login/auth is done and a real shell prompt is showing), so the
///     shell reports its cwd via OSC 7 on every prompt — this is what
///     drives the SFTP pane's cwd-follow. Best-effort: works for bash/zsh
///     (or any shell with pre-existing iTerm2/VSCode-style OSC 7
///     integration already in its rc file); other shells just don't get
///     cwd-follow, and the SFTP pane stays fully usable by hand regardless.
final class PTYSession: LocalProcessTerminalView {
    private(set) var host: Host?
    private var storedPassword: String?
    private var hasSentPassword = false
    private var recentOutput: [UInt8] = []
    private let promptScanWindow = 64

    private var hasInjectedCwdHook = false
    private var cwdHookWorkItem: DispatchWorkItem?

    func connect(to host: Host, resolveJumpHost: @escaping (UUID) -> Host? = { _ in nil }) {
        self.host = host
        if host.authMethod == .password {
            storedPassword = try? KeychainService.read(account: host.keychainAccount)
        }
        let (executable, args) = SSHArgvBuilder.build(for: host, resolveJumpHost: resolveJumpHost)
        startProcess(executable: executable, args: args)
    }

    override func dataReceived(slice: ArraySlice<UInt8>) {
        if !hasSentPassword, let password = storedPassword, !password.isEmpty {
            recentOutput.append(contentsOf: slice)
            if recentOutput.count > promptScanWindow {
                recentOutput.removeFirst(recentOutput.count - promptScanWindow)
            }
            let tail = String(decoding: recentOutput, as: UTF8.self).lowercased()
            if tail.contains("password:") {
                hasSentPassword = true
                process.send(data: ArraySlice(Array((password + "\r").utf8)))
            }
        }
        super.dataReceived(slice: slice)

        if !hasInjectedCwdHook {
            DispatchQueue.main.async { [weak self] in
                self?.scheduleCwdHookInjection()
            }
        }
    }

    /// Debounced on the LAST byte received — cancels and reschedules on
    /// every call, so the hook only fires once output has actually gone
    /// quiet (a real prompt, not mid-banner/mid-password-exchange output).
    private func scheduleCwdHookInjection() {
        guard !hasInjectedCwdHook else { return }
        cwdHookWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.injectCwdHookIfSettled()
        }
        cwdHookWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: workItem)
    }

    private func injectCwdHookIfSettled() {
        guard !hasInjectedCwdHook, let process, process.running else { return }
        hasInjectedCwdHook = true
        process.send(data: ArraySlice(Array(PTYSession.cwdHookScript.utf8)))
    }

    /// One line, leading space (many shells' HIST_IGNORE_SPACE keeps it out
    /// of history), silently no-ops on shells other than bash/zsh.
    private static let cwdHookScript: String = {
        let body = """
         if [ -n "$ZSH_VERSION" ]; then __bhterm_cwd() { printf '\\033]7;file://%s%s\\033\\\\' "$(hostname)" "$PWD"; }; autoload -Uz add-zsh-hook >/dev/null 2>&1 && add-zsh-hook precmd __bhterm_cwd || PROMPT_COMMAND='__bhterm_cwd'; elif [ -n "$BASH_VERSION" ]; then __bhterm_cwd() { printf '\\033]7;file://%s%s\\033\\\\' "$(hostname)" "$PWD"; }; case "$PROMPT_COMMAND" in *__bhterm_cwd*) ;; "") PROMPT_COMMAND='__bhterm_cwd';; *) PROMPT_COMMAND="__bhterm_cwd;$PROMPT_COMMAND";; esac; fi; __bhterm_cwd 2>/dev/null
        """
        return body + "\r"
    }()
}
