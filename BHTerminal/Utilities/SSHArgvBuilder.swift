import Foundation

/// Turns a saved Host into the argv for the real `/usr/bin/ssh` binary.
/// Deliberately does not attempt to pass a password on argv (no sshpass
/// dependency) — password auth still gets ssh's normal interactive prompt
/// in the pty; PTYSession auto-fills it from the Keychain once connected.
enum SSHArgvBuilder {
    /// Throws SSHSafetyError if the host (or its jump host) has a hostname/
    /// username that ssh could misinterpret as an option — see SSHSafety.
    static func build(for host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) throws -> (executable: String, args: [String]) {
        try SSHSafety.validate(host, resolveJumpHost: resolveJumpHost)

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

        // "--" ends ssh option parsing so the destination operand can never
        // be re-read as an option (defense in depth alongside SSHSafety's
        // leading-dash rejection above).
        args.append("--")
        args.append("\(host.username)@\(host.hostname)")

        return ("/usr/bin/ssh", args)
    }

    /// Shared with TunnelArgvBuilder — the -J embedded-port spec format.
    static func jumpSpec(_ host: Host) -> String {
        host.port != 22
            ? "\(host.username)@\(host.hostname):\(host.port)"
            : "\(host.username)@\(host.hostname)"
    }

    static func expandPath(_ path: String) -> String {
        (path as NSString).expandingTildeInPath
    }
}
