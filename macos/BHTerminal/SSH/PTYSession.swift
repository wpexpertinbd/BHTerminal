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
    /// The Keychain secret to auto-fill at ssh's prompt: a password for
    /// password auth, or a key passphrase for private-key auth.
    private var authSecret: String?
    /// Lowercased substring of the prompt that triggers the fill.
    private var authNeedle = "password:"
    /// How many times we may still fill. Password auth = 1; a passphrase can
    /// be asked more than once (a jump host and the target each load the key),
    /// so allow a few — capped so a wrong passphrase can't loop forever.
    private var secretFillsRemaining = 0
    private var recentOutput: [UInt8] = []
    private let promptScanWindow = 128

    private var hasInjectedCwdHook = false
    private var cwdHookWorkItem: DispatchWorkItem?

    private var logFileHandle: FileHandle?

    func connect(to host: Host, resolveJumpHost: @escaping (UUID) -> Host? = { _ in nil }) {
        self.host = host
        applyAppearance()

        let executable: String
        let args: [String]
        do {
            (executable, args) = try SSHArgvBuilder.build(for: host, resolveJumpHost: resolveJumpHost)
        } catch {
            // Refuse to spawn ssh with an unsafe hostname/username — show the
            // reason in the pane instead of silently connecting.
            let message = (error as? LocalizedError)?.errorDescription ?? "\(error)"
            feed(text: "\r\n\u{001B}[31mCannot connect: \(message)\u{001B}[0m\r\n")
            return
        }

        switch host.authMethod {
        case .password:
            if let password = try? KeychainService.read(account: host.keychainAccount), !password.isEmpty {
                authSecret = password
                authNeedle = "password:"
                secretFillsRemaining = 1
            }
        case .privateKey:
            // Auto-fill the saved key passphrase at ssh's "Enter passphrase
            // for key '…':" prompt.
            if let passphrase = try? KeychainService.read(account: host.passphraseAccount), !passphrase.isEmpty {
                authSecret = passphrase
                authNeedle = "passphrase for"
                secretFillsRemaining = 3
            }
        case .agent:
            break
        }
        startProcess(executable: executable, args: args)
    }

    private func applyAppearance() {
        font = AppSettings.shared.font
        if let fg = AppSettings.shared.theme.foreground, let bg = AppSettings.shared.theme.background {
            nativeForegroundColor = fg
            nativeBackgroundColor = bg
        } else {
            configureNativeColors()
        }
    }

    /// Types a command into the pty as if the user typed it, followed by
    /// Enter — used for snippet dispatch and (internally) the password/cwd
    /// hooks below.
    func sendCommand(_ text: String) {
        process.send(data: ArraySlice(Array((text + "\r").utf8)))
    }

    // MARK: - Right-click Copy / Paste / Select

    /// SwiftTerm's TerminalView already implements copy/paste/selectAll and
    /// exposes `getSelection()`; it just doesn't provide a context menu. NSView
    /// calls `menu(for:)` on right-click, so wire one up.
    ///
    /// autoenablesItems is turned OFF: SwiftTerm implements
    /// `validateUserInterfaceItem`, which AppKit would call for these custom
    /// items and — not recognising the selectors — disable them all. So enable
    /// them ourselves.
    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        menu.autoenablesItems = false
        let hasSelection = !(getSelection()?.isEmpty ?? true)
        let hasClipboardText = NSPasteboard.general.string(forType: .string)?.isEmpty == false

        addContextItem(menu, "Copy", #selector(contextCopy), enabled: hasSelection)
        addContextItem(menu, "Paste", #selector(contextPaste), enabled: hasClipboardText)
        menu.addItem(.separator())
        addContextItem(menu, "Select All", #selector(contextSelectAll), enabled: true)
        addContextItem(menu, "Copy All", #selector(contextCopyAll), enabled: true)
        return menu
    }

    private func addContextItem(_ menu: NSMenu, _ title: String, _ action: Selector, enabled: Bool) {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        item.isEnabled = enabled
        menu.addItem(item)
    }

    @objc private func contextCopy() { copy(self) }
    @objc private func contextPaste() { paste(self) }
    @objc private func contextSelectAll() { selectAll(nil) }
    @objc private func contextCopyAll() {
        selectAll(nil)
        copy(self)
    }

    func startLogging(to url: URL) throws {
        stopLogging()
        if !FileManager.default.fileExists(atPath: url.path) {
            // 0600 — a session transcript can contain sensitive output, so
            // don't leave it group/world-readable to other local accounts.
            FileManager.default.createFile(atPath: url.path, contents: nil,
                                           attributes: [.posixPermissions: 0o600])
        }
        let handle = try FileHandle(forWritingTo: url)
        handle.seekToEndOfFile()
        logFileHandle = handle
    }

    func stopLogging() {
        try? logFileHandle?.close()
        logFileHandle = nil
    }

    override func dataReceived(slice: ArraySlice<UInt8>) {
        if secretFillsRemaining > 0, let secret = authSecret {
            recentOutput.append(contentsOf: slice)
            if recentOutput.count > promptScanWindow {
                recentOutput.removeFirst(recentOutput.count - promptScanWindow)
            }
            let tail = String(decoding: recentOutput, as: UTF8.self).lowercased()
            if tail.contains(authNeedle) {
                secretFillsRemaining -= 1
                recentOutput.removeAll() // don't re-match the same prompt still in the window
                process.send(data: ArraySlice(Array((secret + "\r").utf8)))
            }
        }

        logFileHandle?.write(Data(slice))

        super.dataReceived(slice: slice)

        if !hasInjectedCwdHook {
            DispatchQueue.main.async { [weak self] in
                self?.scheduleCwdHookInjection()
            }
        }
    }

    deinit {
        logFileHandle?.closeFile()
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
        // Output has settled into a real shell prompt → auth is done. Drop the
        // secret from memory and stop watching for a prompt to fill.
        authSecret = nil
        secretFillsRemaining = 0
        recentOutput = []
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
