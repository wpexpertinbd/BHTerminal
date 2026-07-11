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

        var args = connectionOptions(for: host, resolveJumpHost: resolveJumpHost)

        // Become the multiplexing master so the SFTP pane can reuse this
        // authenticated connection (keys/agent/passphrase/2FA), with the
        // socket persisting briefly after the terminal closes.
        args += ["-o", "ControlMaster=auto",
                 "-o", "ControlPath=\(controlPath)",
                 "-o", "ControlPersist=120"]

        // "--" ends ssh option parsing so the destination operand can never
        // be re-read as an option (defense in depth alongside SSHSafety's
        // leading-dash rejection above).
        args.append("--")
        args.append(destination(for: host))

        return ("/usr/bin/ssh", args)
    }

    /// argv for opening the SFTP subsystem over real ssh, reusing the
    /// terminal's multiplexed connection when it exists. `BatchMode=yes` means
    /// it never blocks on a passphrase prompt (it has no tty): if the master
    /// isn't up yet it fails fast and SFTPConnection retries until it is.
    static func buildSFTP(for host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) throws -> (executable: String, args: [String]) {
        try SSHSafety.validate(host, resolveJumpHost: resolveJumpHost)

        var args = connectionOptions(for: host, resolveJumpHost: resolveJumpHost)
        args += ["-o", "ControlPath=\(controlPath)",
                 "-o", "BatchMode=yes",
                 "-s", "--", destination(for: host), "sftp"]

        return ("/usr/bin/ssh", args)
    }

    // MARK: - Shared pieces

    /// port / key / jump options common to both the terminal and SFTP argv.
    private static func connectionOptions(for host: Host, resolveJumpHost: (UUID) -> Host?) -> [String] {
        var args: [String] = []
        if host.port != 22 {
            args += ["-p", String(host.port)]
        }
        if case .privateKey(let path) = host.authMethod {
            args += ["-i", expandPath(path)]
        }
        if let jumpID = host.jumpHostID, let jumpHost = resolveJumpHost(jumpID) {
            args += ["-J", jumpSpec(jumpHost)]
        }
        return args
    }

    private static func destination(for host: Host) -> String {
        "\(host.username)@\(host.hostname)"
    }

    /// Control socket shared between the terminal and SFTP for a host. `%C` is
    /// ssh's own hash of (localhost, remotehost, port, user) — short, and it
    /// resolves identically for both invocations, so they share one master.
    private static let controlPath = "/tmp/bht-%C"

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
