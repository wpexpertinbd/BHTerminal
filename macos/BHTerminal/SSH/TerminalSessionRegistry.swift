import AppKit

/// Owns the live PTYSession (and its container view) for every terminal pane,
/// for the lifetime of the app rather than the lifetime of a SwiftUI view.
///
/// This exists because SwiftUI freely tears down and rebuilds
/// NSViewRepresentables — switching tabs used to destroy the outgoing tab's
/// `SplitPaneView`, whose `dismantleNSView` terminated the ssh processes. The
/// result was that leaving a tab killed that server's session and coming back
/// re-ran ssh as a brand-new login. Sessions now belong here, so view teardown
/// is harmless: a connection is only ever terminated when its pane or tab is
/// explicitly closed (or the app quits).
@MainActor
final class TerminalSessionRegistry {
    static let shared = TerminalSessionRegistry()

    private var sessions: [UUID: PTYSession] = [:]
    private var containers: [UUID: NSView] = [:]

    /// Bumped when a pane's session is replaced (reconnect), so SplitPaneView
    /// knows to swap in the new container view.
    private var generations: [UUID: Int] = [:]

    private init() {}

    func generation(for paneID: UUID) -> Int { generations[paneID] ?? 0 }

    func existingSession(for paneID: UUID) -> PTYSession? { sessions[paneID] }

    func container(for paneID: UUID) -> NSView? { containers[paneID] }

    /// The pane's session, connecting it on first request. Repeat calls return
    /// the same live session — never a second ssh process for the same pane.
    func session(for paneID: UUID, host: Host,
                 resolveJumpHost: @escaping (UUID) -> Host?) -> PTYSession {
        if let existing = sessions[paneID] { return existing }
        let session = PTYSession(frame: .zero)
        sessions[paneID] = session
        // connect() applies the theme, so wrap it only afterwards — the
        // container's margin colour is taken from the session.
        session.connect(to: host, resolveJumpHost: resolveJumpHost)
        containers[paneID] = PaddedTerminalContainer(session: session)
        return session
    }

    /// Tears down the old ssh and starts a fresh one for the same pane — used
    /// by the pane's Reconnect button after a dropped connection.
    func reconnect(paneID: UUID, host: Host,
                   resolveJumpHost: @escaping (UUID) -> Host?) {
        terminate(paneID: paneID)
        generations[paneID] = generation(for: paneID) + 1

        // Kill the shared connection first, unless another pane is still using
        // it: after a network drop the master often survives locally with a dead
        // TCP connection, and reusing it is why reconnecting used to hang until
        // the app was restarted.
        if !hasOtherLiveSession(forHostID: host.id, excluding: paneID) {
            SSHArgvBuilder.terminateControlMaster(for: host, resolveJumpHost: resolveJumpHost)
        }

        _ = session(for: paneID, host: host, resolveJumpHost: resolveJumpHost)
    }

    /// Whether some other pane still has a live ssh to the same host (so its
    /// shared connection must not be torn down).
    func hasOtherLiveSession(forHostID hostID: UUID, excluding paneID: UUID) -> Bool {
        sessions.contains { id, session in
            id != paneID && session.host?.id == hostID && session.isRunning
        }
    }

    func terminate(paneID: UUID) {
        sessions[paneID]?.stopLogging()
        sessions[paneID]?.terminate()
        sessions.removeValue(forKey: paneID)
        containers[paneID]?.removeFromSuperview()
        containers.removeValue(forKey: paneID)
    }

    func terminateAll() {
        for paneID in sessions.keys { terminate(paneID: paneID) }
    }
}
