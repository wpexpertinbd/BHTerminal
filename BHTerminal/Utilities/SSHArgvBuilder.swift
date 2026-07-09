import Foundation

/// Turns a saved Host into the argv for the real `/usr/bin/ssh` binary.
/// Deliberately does not attempt to pass a password on argv (no sshpass
/// dependency) — password auth still gets ssh's normal interactive prompt
/// in the pty; PTYSession auto-fills it from the Keychain once connected.
enum SSHArgvBuilder {
    static func build(for host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) -> (executable: String, args: [String]) {
        var args: [String] = []

        if host.port != 22 {
            args += ["-p", String(host.port)]
        }

        if case .privateKey(let path) = host.authMethod {
            args += ["-i", expandPath(path)]
        }

        if let jumpID = host.jumpHostID, let jumpHost = resolveJumpHost(jumpID) {
            // -J's spec embeds the port inline (user@host:port); this is
            // NOT valid syntax for the primary destination below, which
            // takes its port via a separate -p flag instead.
            args += ["-J", jumpSpec(jumpHost)]
        }

        args.append("\(host.username)@\(host.hostname)")

        return ("/usr/bin/ssh", args)
    }

    private static func jumpSpec(_ host: Host) -> String {
        host.port != 22
            ? "\(host.username)@\(host.hostname):\(host.port)"
            : "\(host.username)@\(host.hostname)"
    }

    private static func expandPath(_ path: String) -> String {
        (path as NSString).expandingTildeInPath
    }
}
