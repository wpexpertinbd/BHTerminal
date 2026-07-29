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
        // Keepalives are what let a dropped network actually END the session
        // instead of wedging it. Without them the ControlMaster process
        // survives with a dead TCP connection (ControlPersist keeps it around),
        // and every later session — including a manual reconnect — multiplexes
        // onto that corpse and hangs until the kernel's TCP timeout. ssh now
        // notices within ~45s and exits, so reconnects work.
        var args: [String] = ["-o", "ServerAliveInterval=15",
                              "-o", "ServerAliveCountMax=3",
                              "-o", "TCPKeepAlive=yes",
                              "-o", "ConnectTimeout=20"]
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
    ///
    /// It lives in a per-user 0700 directory, NOT world-writable /tmp: `%C` is
    /// predictable, so a socket in /tmp could be squatted by another local user
    /// to hijack (MITM) the multiplexed SSH session or to deny multiplexing.
    /// A 0700 dir under $HOME can't be entered by other UIDs.
    private static var controlPath: String {
        let dir = (NSHomeDirectory() as NSString).appendingPathComponent(".bhterminal/cm")
        let fm = FileManager.default
        try? fm.createDirectory(atPath: dir, withIntermediateDirectories: true,
                                attributes: [.posixPermissions: 0o700])
        // Enforce 0700 even if the dir (or its parent) pre-existed with looser
        // permissions — the socket's confidentiality depends on it.
        try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir)
        try? fm.setAttributes([.posixPermissions: 0o700],
                              ofItemAtPath: (NSHomeDirectory() as NSString).appendingPathComponent(".bhterminal"))
        return (dir as NSString).appendingPathComponent("%C")
    }

    /// Force-closes the shared connection for a host (`ssh -O exit`), so the
    /// next connect builds a brand-new one.
    ///
    /// A user-initiated reconnect must not reuse the existing master: after a
    /// network drop the master can still be alive locally (its socket answers,
    /// so `-O check` passes) while its TCP connection is dead, which is exactly
    /// why "reconnect" used to do nothing until the app was restarted. Callers
    /// must first ensure no other live pane is sharing this host's connection.
    /// Best-effort and synchronous-but-brief; failure just means no master.
    static func terminateControlMaster(for host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) {
        guard (try? SSHSafety.validate(host, resolveJumpHost: resolveJumpHost)) != nil else { return }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
        process.arguments = ["-o", "ControlPath=\(controlPath)",
                             "-O", "exit", "--", destination(for: host)]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            // `-O exit` only talks to the local control socket (no network), so
            // it returns in milliseconds — or fails instantly when there's no
            // master. The cap is purely so a wedged socket can't stall the
            // Reconnect click.
            let deadline = Date().addingTimeInterval(2)
            while process.isRunning && Date() < deadline {
                usleep(20_000)
            }
            if process.isRunning { process.terminate() }
        } catch {
            // No ssh / nothing to close — the fresh connect will handle it.
        }
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
