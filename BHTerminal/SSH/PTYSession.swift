import AppKit
import SwiftTerm

/// Subclasses SwiftTerm's `LocalProcessTerminalView` — its own docs call
/// this out as the sanctioned extension point ("If you want additional
/// control ... you can subclass this and override the methods").
///
/// The only thing added here is auto-filling a Keychain-stored password at
/// ssh's own interactive password prompt, by watching the tail of the raw
/// output for a "password:" cue before it's fed to the terminal. Everything
/// else is unmodified real `ssh` in a pty — no protocol reimplementation.
final class PTYSession: LocalProcessTerminalView {
    private(set) var host: Host?
    private var storedPassword: String?
    private var hasSentPassword = false
    private var recentOutput: [UInt8] = []
    private let promptScanWindow = 64

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
    }
}
