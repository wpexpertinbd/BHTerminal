import Foundation

/// Turns a TunnelRule + its Host into argv for a background `ssh -N`
/// process — same reasoning as SSHArgvBuilder: reuse real ssh's own
/// battle-tested port-forwarding rather than reimplementing it.
enum TunnelArgvBuilder {
    /// Throws SSHSafetyError if the host (or its jump host) has an unsafe
    /// hostname/username — see SSHSafety.
    static func build(for tunnel: TunnelRule, host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) throws -> (executable: String, args: [String]) {
        try SSHSafety.validate(host, resolveJumpHost: resolveJumpHost)

        var args = ["-N"] // no remote command — this ssh process only forwards

        if host.port != 22 {
            args += ["-p", String(host.port)]
        }

        if case .privateKey(let path) = host.authMethod {
            args += ["-i", SSHArgvBuilder.expandPath(path)]
        }

        if let jumpID = host.jumpHostID, let jumpHost = resolveJumpHost(jumpID) {
            args += ["-J", SSHArgvBuilder.jumpSpec(jumpHost)]
        }

        switch tunnel.kind {
        case .local:
            args += ["-L", "\(tunnel.listenHost):\(tunnel.listenPort):\(tunnel.destHost):\(tunnel.destPort)"]
        case .remote:
            args += ["-R", "\(tunnel.listenHost):\(tunnel.listenPort):\(tunnel.destHost):\(tunnel.destPort)"]
        case .dynamic:
            args += ["-D", "\(tunnel.listenHost):\(tunnel.listenPort)"]
        }

        // End option parsing before the destination operand (see SSHArgvBuilder).
        args.append("--")
        args.append("\(host.username)@\(host.hostname)")

        return ("/usr/bin/ssh", args)
    }
}
