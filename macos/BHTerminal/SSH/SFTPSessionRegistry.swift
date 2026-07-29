import Foundation

/// One live SFTP session per host, kept for the app's lifetime — the file-pane
/// counterpart of TerminalSessionRegistry.
///
/// A single shared connection meant every terminal-tab switch tore the SFTP
/// channel down and rebuilt it against the newly-selected host, so the browser
/// spent its time reconnecting and forgot which directory you were in. Holding
/// one connection per host makes switching tabs instant: the pane just shows
/// that host's existing session, still sitting in the folder you left it in.
@MainActor
@Observable
final class SFTPSessionRegistry {
    static let shared = SFTPSessionRegistry()

    private(set) var connections: [UUID: SFTPConnection] = [:]

    /// Mirrors the "Follow Terminal" checkbox onto sessions created later, so
    /// the setting doesn't silently differ per host.
    private var followTerminal = false

    private init() {}

    func existing(for hostID: UUID) -> SFTPConnection? { connections[hostID] }

    /// This host's session, created (not yet connected) on first request.
    func connection(for host: Host) -> SFTPConnection {
        if let existing = connections[host.id] { return existing }
        let connection = SFTPConnection()
        connection.setFollowTerminal(followTerminal)
        connections[host.id] = connection
        return connection
    }

    /// Connects this host's session unless it's already connected//connecting —
    /// the point of the registry is that revisiting a tab is a no-op.
    func activate(host: Host, resolveJumpHost: @escaping (UUID) -> Host?) {
        let connection = connection(for: host)
        guard connection.connectedHost?.id != host.id, !connection.isLoading else { return }
        Task { await connection.connect(to: host, resolveJumpHost: resolveJumpHost) }
    }

    /// The checkbox is a single user-level preference, so apply it everywhere
    /// rather than letting each host drift out of sync.
    func setFollowTerminalForAll(_ on: Bool) {
        followTerminal = on
        for connection in connections.values {
            connection.setFollowTerminal(on)
        }
    }

    func disconnect(hostID: UUID) async {
        guard let connection = connections[hostID] else { return }
        await connection.disconnect()
        connections.removeValue(forKey: hostID)
    }

    /// Drops every session whose host no longer has an open terminal — SFTP
    /// rides the terminal's authenticated connection, so it can't outlive it.
    func pruneSessions(keepingHostIDs keep: Set<UUID>) async {
        for hostID in connections.keys where !keep.contains(hostID) {
            await disconnect(hostID: hostID)
        }
    }

    func disconnectAll() async {
        for hostID in connections.keys {
            await disconnect(hostID: hostID)
        }
    }

    /// Synchronous variant for app termination (an async Task wouldn't run).
    func closeAllForTermination() {
        for connection in connections.values {
            connection.closeForTermination()
        }
        connections.removeAll()
    }
}
