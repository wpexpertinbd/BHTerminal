import Foundation

enum SFTPConnectionError: Error, LocalizedError {
    case couldNotOpen

    var errorDescription: String? {
        switch self {
        case .couldNotOpen:
            return "Couldn't open SFTP. Make sure the terminal for this host has finished connecting (SFTP reuses that authenticated connection), then try again from the ↻ button."
        }
    }
}

/// The file browser's SFTP session. It runs the SFTP subsystem over your
/// **real ssh** (`ssh … -s -- host sftp`) and reuses the terminal's
/// multiplexed connection, so it authenticates with keys, ssh-agent,
/// passphrase-protected keys, 2FA, and ProxyJump exactly like the terminal —
/// no separate password. The structured file operations come from
/// SFTPProtocolClient speaking SFTP v3 over that process's pipes.
@MainActor
@Observable
final class SFTPConnection {
    private(set) var connectedHost: Host?
    private(set) var currentPath: String = "."
    private(set) var entries: [SFTPEntry] = []
    private(set) var isLoading = false
    private(set) var errorMessage: String?
    /// Shown while waiting for the terminal to finish authenticating so the
    /// shared connection exists (e.g. the user is typing a key passphrase).
    private(set) var statusMessage: String?
    /// When on, the browser jumps to the terminal's current directory as you
    /// `cd` around. Off by default (opt-in via the footer checkbox).
    private(set) var followTerminal = false

    private var client: SFTPProtocolClient?
    private var generation = 0
    /// Latest cwd reported by the terminal (OSC 7), remembered even while
    /// not following, so toggling "Follow Terminal" on jumps there immediately.
    private var lastTerminalPath: String?

    /// How long to keep retrying the SFTP subsystem while the terminal's
    /// master connection comes up (covers the user typing a passphrase / 2FA).
    private let connectTimeout: TimeInterval = 45
    private let retryInterval: TimeInterval = 1.2

    func connect(to host: Host, resolveJumpHost: @escaping (UUID) -> Host? = { _ in nil }) async {
        generation += 1
        let gen = generation

        await teardown()
        connectedHost = host
        entries = []
        errorMessage = nil
        currentPath = "."
        lastTerminalPath = nil
        isLoading = true
        statusMessage = "Connecting…"
        defer { if gen == generation { isLoading = false; statusMessage = nil } }

        let executable: String, args: [String]
        do {
            (executable, args) = try SSHArgvBuilder.buildSFTP(for: host, resolveJumpHost: resolveJumpHost)
        } catch {
            errorMessage = describeError(error)
            connectedHost = nil
            return
        }

        let deadline = Date().addingTimeInterval(connectTimeout)
        var attempted = false
        while Date() < deadline {
            guard gen == generation else { return } // superseded by a newer connect
            do {
                let newClient = try await SFTPProtocolClient.launch(executable: executable, args: args)
                guard gen == generation else { newClient.close(); return }
                client = newClient
                await load(path: ".", showLoading: false)
                return
            } catch {
                // Almost always: the terminal's master connection isn't up yet
                // (BatchMode ssh can't prompt for the passphrase itself). Wait
                // and retry until it is, or we time out.
                if attempted { statusMessage = "Waiting for the terminal to authenticate…" }
                attempted = true
                try? await Task.sleep(nanoseconds: UInt64(retryInterval * 1_000_000_000))
            }
        }

        guard gen == generation else { return }
        errorMessage = SFTPConnectionError.couldNotOpen.localizedDescription
        connectedHost = nil
    }

    func disconnect() async {
        generation += 1
        await teardown()
        connectedHost = nil
        entries = []
        currentPath = "."
        lastTerminalPath = nil
        errorMessage = nil
        statusMessage = nil
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

    /// Driven by the terminal's OSC 7 cwd-follow — `path` is already absolute.
    /// Remembers the path always, but only navigates when "Follow Terminal" is on.
    func navigateToAbsolutePath(_ path: String) async {
        lastTerminalPath = path
        guard followTerminal, client != nil, path != currentPath else { return }
        await load(path: path, showLoading: false)
    }

    /// Toggled by the browser's "Follow Terminal" checkbox. Turning it on jumps
    /// to the terminal's last-known directory right away.
    func setFollowTerminal(_ on: Bool) {
        followTerminal = on
        if on, let path = lastTerminalPath, path != currentPath, client != nil {
            Task { await load(path: path, showLoading: false) }
        }
    }

    func createDirectory(name: String) async {
        guard let client else { return }
        do {
            try await client.makeDirectory(join(currentPath, name))
            await refresh()
        } catch {
            errorMessage = "Could not create folder: \(describeError(error))"
        }
    }

    func rename(_ entry: SFTPEntry, to newName: String) async {
        guard let client else { return }
        do {
            try await client.rename(from: join(currentPath, entry.name), to: join(currentPath, newName))
            await refresh()
        } catch {
            errorMessage = "Could not rename: \(describeError(error))"
        }
    }

    func delete(_ entry: SFTPEntry) async {
        guard let client else { return }
        do {
            if entry.isDirectory {
                try await client.removeDirectory(join(currentPath, entry.name))
            } else {
                try await client.removeFile(join(currentPath, entry.name))
            }
            await refresh()
        } catch {
            errorMessage = "Could not delete: \(describeError(error))"
        }
    }

    func download(_ entry: SFTPEntry, to localURL: URL) async {
        guard let client, !entry.isDirectory else { return }
        do {
            // Stream to disk rather than buffering the whole file in memory,
            // so a huge (or malicious never-EOF) file can't exhaust RAM.
            FileManager.default.createFile(atPath: localURL.path, contents: nil)
            let handle = try FileHandle(forWritingTo: localURL)
            defer { try? handle.close() }
            try await client.downloadFile(join(currentPath, entry.name), to: handle)
        } catch {
            errorMessage = "Download failed: \(describeError(error))"
        }
    }

    func upload(from localURL: URL, as name: String? = nil) async {
        guard let client else { return }
        do {
            let data = try Data(contentsOf: localURL)
            try await client.writeFile(join(currentPath, name ?? localURL.lastPathComponent), data: data)
            await refresh()
        } catch {
            errorMessage = "Upload failed: \(describeError(error))"
        }
    }

    // MARK: - Internals

    private func load(path: String, showLoading: Bool) async {
        guard let client else { return }
        if showLoading { isLoading = true }
        defer { if showLoading { isLoading = false } }

        do {
            let resolved = try await client.realpath(path)
            let listing = try await client.listDirectory(resolved)
            entries = listing
                .filter { $0.filename != "." && $0.filename != ".." }
                .map(SFTPEntry.init(entry:))
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
        client?.close()
        client = nil
    }

    private func join(_ base: String, _ name: String) -> String {
        base.hasSuffix("/") ? base + name : base + "/" + name
    }

    private func describeError(_ error: Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? "\(error)"
    }
}
