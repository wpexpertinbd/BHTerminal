import Foundation
import Citadel
import NIOCore

enum SFTPConnectionError: Error, LocalizedError {
    case unsupportedAuthMethod
    case missingPassword
    case untrustedHostKey

    var errorDescription: String? {
        switch self {
        case .unsupportedAuthMethod:
            return "SFTP browsing currently supports password-authenticated hosts only. Key/agent auth still works fine in the terminal — switch this host to Password auth to browse its files here."
        case .missingPassword:
            return "No password saved in Keychain for this host."
        case .untrustedHostKey:
            return "This host's key isn't in ~/.ssh/known_hosts yet. Connect once via a terminal tab first (so ssh can record and verify it), then retry."
        }
    }
}

/// A Citadel SFTP session, independent of PTYSession's real-ssh pty — the
/// file browser needs structured list/read/write calls, not scraped `sftp`
/// CLI output. Host-key trust goes through KnownHostsValidator, the same
/// ~/.ssh/known_hosts real ssh uses, rather than blindly accepting keys.
///
/// Auth is password-only for now: Citadel's client wants a typed private
/// key (Curve25519.Signing.PrivateKey etc.), and there's no built-in loader
/// from an OpenSSH private-key file, let alone one that speaks the
/// ssh-agent protocol. Hand-rolling that decode (and, for encrypted keys,
/// bcrypt_pbkdf + AES) is real security-sensitive parsing code that
/// deserves its own careful pass rather than being rushed in here — the
/// terminal already handles every auth method fine via real ssh.
@MainActor
@Observable
final class SFTPConnection {
    private(set) var connectedHost: Host?
    private(set) var currentPath: String = "."
    private(set) var entries: [SFTPEntry] = []
    private(set) var isLoading = false
    private(set) var errorMessage: String?

    private var sshClient: SSHClient?
    private var sftp: SFTPClient?

    func connect(to host: Host) async {
        errorMessage = nil

        guard host.authMethod == .password else {
            errorMessage = SFTPConnectionError.unsupportedAuthMethod.localizedDescription
            connectedHost = nil
            entries = []
            return
        }
        guard let password = try? KeychainService.read(account: host.keychainAccount), !password.isEmpty else {
            errorMessage = SFTPConnectionError.missingPassword.localizedDescription
            connectedHost = nil
            entries = []
            return
        }

        isLoading = true
        defer { isLoading = false }

        await teardown()

        do {
            let validator = KnownHostsValidator(hostname: host.hostname, port: host.port)
            let client = try await SSHClient.connect(
                host: host.hostname,
                port: host.port,
                authenticationMethod: .passwordBased(username: host.username, password: password),
                hostKeyValidator: .custom(validator),
                reconnect: .never
            )
            let sftpClient = try await client.openSFTP()
            sshClient = client
            sftp = sftpClient
            connectedHost = host
            await load(path: ".", showLoading: false)
        } catch {
            errorMessage = describeError(error)
            connectedHost = nil
        }
    }

    func disconnect() async {
        await teardown()
        connectedHost = nil
        entries = []
        currentPath = "."
        errorMessage = nil
    }

    func refresh() async {
        await load(path: currentPath, showLoading: true)
    }

    func navigate(to entry: SFTPEntry) async {
        guard entry.isDirectory else { return }
        await load(path: join(currentPath, entry.name), showLoading: true)
    }

    func navigateUp() async {
        let parent = (currentPath as NSString).deletingLastPathComponent
        await load(path: parent.isEmpty ? "/" : parent, showLoading: true)
    }

    /// Driven by the terminal's OSC 7 cwd-follow — `path` is already
    /// absolute (from PTYSession's shell hook), unlike `navigate(to:)`
    /// which resolves relative to `currentPath`.
    func navigateToAbsolutePath(_ path: String) async {
        guard sftp != nil, path != currentPath else { return }
        await load(path: path, showLoading: false)
    }

    func createDirectory(name: String) async {
        guard let sftp else { return }
        do {
            try await sftp.createDirectory(atPath: join(currentPath, name))
            await refresh()
        } catch {
            errorMessage = "Could not create folder: \(describeError(error))"
        }
    }

    func rename(_ entry: SFTPEntry, to newName: String) async {
        guard let sftp else { return }
        do {
            try await sftp.rename(at: join(currentPath, entry.name), to: join(currentPath, newName))
            await refresh()
        } catch {
            errorMessage = "Could not rename: \(describeError(error))"
        }
    }

    func delete(_ entry: SFTPEntry) async {
        guard let sftp else { return }
        do {
            if entry.isDirectory {
                try await sftp.rmdir(at: join(currentPath, entry.name))
            } else {
                try await sftp.remove(at: join(currentPath, entry.name))
            }
            await refresh()
        } catch {
            errorMessage = "Could not delete: \(describeError(error))"
        }
    }

    func download(_ entry: SFTPEntry, to localURL: URL) async {
        guard let sftp, !entry.isDirectory else { return }
        do {
            let remotePath = join(currentPath, entry.name)
            let buffer = try await sftp.withFile(filePath: remotePath, flags: .read) { file in
                try await file.readAll()
            }
            try Data(buffer.readableBytesView).write(to: localURL)
        } catch {
            errorMessage = "Download failed: \(describeError(error))"
        }
    }

    func upload(from localURL: URL, as name: String? = nil) async {
        guard let sftp else { return }
        do {
            let data = try Data(contentsOf: localURL)
            var mutableBuffer = ByteBuffer()
            mutableBuffer.writeBytes(data)
            let payload = mutableBuffer
            let remotePath = join(currentPath, name ?? localURL.lastPathComponent)
            try await sftp.withFile(filePath: remotePath, flags: [.write, .create, .truncate]) { file in
                try await file.write(payload)
            }
            await refresh()
        } catch {
            errorMessage = "Upload failed: \(describeError(error))"
        }
    }

    // MARK: - Internals

    private func load(path: String, showLoading: Bool) async {
        guard let sftp else { return }
        if showLoading { isLoading = true }
        defer { if showLoading { isLoading = false } }

        do {
            let resolved = try await sftp.getRealPath(atPath: path)
            let names = try await sftp.listDirectory(atPath: resolved)
            entries = names
                .flatMap(\.components)
                .filter { $0.filename != "." && $0.filename != ".." }
                .map(SFTPEntry.init(component:))
                .sorted { lhs, rhs in
                    if lhs.isDirectory != rhs.isDirectory { return lhs.isDirectory }
                    return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
                }
            currentPath = resolved
            errorMessage = nil
        } catch {
            errorMessage = "Could not list \(path): \(describeError(error))"
        }
    }

    private func teardown() async {
        if let sftp {
            try? await sftp.close()
        }
        if let sshClient {
            try? await sshClient.close()
        }
        sftp = nil
        sshClient = nil
    }

    private func join(_ base: String, _ name: String) -> String {
        base.hasSuffix("/") ? base + name : base + "/" + name
    }

    private func describeError(_ error: Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? "\(error)"
    }
}
