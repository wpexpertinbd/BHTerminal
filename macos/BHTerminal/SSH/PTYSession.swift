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
    private let promptScanWindow = 256

    /// Fragments that mean ssh is sitting at a prompt waiting for a human,
    /// rather than showing shell output. Used to tell "the connection went
    /// quiet because it's ready" apart from "…because it's waiting for input".
    private static let authPromptNeedles = [
        "passphrase for", "password:", "password for", "verification code",
        "authentication code", "one-time password", "passcode", "token:",
        "otp:", "(yes/no", "authenticity of host"
    ]

    /// Output has settled into a real shell prompt → login/auth finished. This
    /// is the silent signal that drives the SFTP pane's auto-connect (it needs
    /// no shell hook, so it fires regardless of "Follow Terminal").
    private var hasSignaledReady = false
    var onReady: (() -> Void)?

    private var settleWorkItem: DispatchWorkItem?

    private var logFileHandle: FileHandle?

    /// Whether this pane's ssh is still alive (false once it exits — e.g. a
    /// dropped network or a logout).
    var isRunning: Bool {
        guard let process else { return false }
        return process.running
    }

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
        // Tracked until the shell is up, so the settle check below can tell a
        // waiting prompt from a finished login.
        if !hasSignaledReady {
            recentOutput.append(contentsOf: slice)
            if recentOutput.count > promptScanWindow {
                recentOutput.removeFirst(recentOutput.count - promptScanWindow)
            }
            fillSecretIfPrompted()
        }

        logFileHandle?.write(Data(slice))

        super.dataReceived(slice: slice)

        if !hasSignaledReady {
            DispatchQueue.main.async { [weak self] in
                self?.scheduleSettleCheck()
            }
        }
    }

    deinit {
        logFileHandle?.closeFile()
    }

    /// Debounced on the LAST byte received — cancels and reschedules on
    /// every call, so it only fires once output has actually gone quiet (a
    /// real prompt, not mid-banner/mid-password-exchange output).
    private func scheduleSettleCheck() {
        guard !hasSignaledReady else { return }
        settleWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.handleOutputSettled()
        }
        settleWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: workItem)
    }

    /// Types the Keychain secret in as soon as ssh's matching prompt shows.
    private func fillSecretIfPrompted() {
        guard secretFillsRemaining > 0, let secret = authSecret, let process else { return }
        let tail = String(decoding: recentOutput, as: UTF8.self).lowercased()
        guard tail.contains(authNeedle) else { return }
        secretFillsRemaining -= 1
        recentOutput.removeAll() // don't re-match the same prompt still in the window
        process.send(data: ArraySlice(Array((secret + "\r").utf8)))
    }

    /// Whether the last line of output is a prompt waiting for input. Prompts
    /// don't end in a newline, so the tail ends with ":" / "?" / "(yes/no…".
    /// Pure so it can be unit-tested without a live ssh.
    static func looksLikeAuthPrompt(_ output: String) -> Bool {
        let lines = output.split(separator: "\n", omittingEmptySubsequences: false)
        let lastLine = (lines.last.map(String.init) ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard !lastLine.isEmpty else { return false }
        guard lastLine.hasSuffix(":") || lastLine.hasSuffix("?")
                || lastLine.contains("(yes/no") else { return false }
        return authPromptNeedles.contains { lastLine.contains($0) }
    }

    private var isWaitingOnAuthPrompt: Bool {
        Self.looksLikeAuthPrompt(String(decoding: recentOutput, as: UTF8.self))
    }

    private func handleOutputSettled() {
        guard !hasSignaledReady, let process, process.running else { return }

        // An output lull is indistinguishable from a prompt waiting for input,
        // so check for one before declaring the login finished. Assuming "quiet
        // == ready" broke any server that took longer than the debounce to ask
        // for a key passphrase: the stored secret was wiped before ssh had even
        // asked, so nothing answered the prompt (session hung forever) and the
        // SFTP pane was told to connect to a session that wasn't up yet.
        if isWaitingOnAuthPrompt {
            fillSecretIfPrompted()
            // Answering it — or the user typing — produces output, which
            // re-arms this check from dataReceived. No polling needed.
            return
        }

        hasSignaledReady = true
        // A real shell prompt is showing → auth is done. Drop the secret from
        // memory and stop watching for a prompt to fill.
        authSecret = nil
        secretFillsRemaining = 0
        recentOutput = []
        // Tell the SFTP pane the shared connection is up.
        onReady?()
    }
}
