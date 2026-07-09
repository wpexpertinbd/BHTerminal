import SwiftUI
import AppKit
import SwiftTerm

/// Wraps an NSSplitView whose arranged subviews are PTYSession instances,
/// one per TerminalTabPane. The Coordinator caches sessions by pane id so
/// that unrelated SwiftUI re-renders (e.g. sidebar edits elsewhere in the
/// window) never tear down and recreate a live SSH connection — a pane's
/// PTYSession is only created once and only destroyed when its id actually
/// leaves the `panes` array.
struct SplitPaneView: NSViewRepresentable {
    let panes: [TerminalTabPane]
    let axis: Axis
    let store: SessionStore
    var onPaneExit: (UUID) -> Void = { _ in }
    var onCwdChange: (UUID, String) -> Void = { _, _ in }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSSplitView {
        let splitView = NSSplitView()
        splitView.dividerStyle = .thin
        splitView.isVertical = (axis == .horizontal)
        context.coordinator.owner = self
        return splitView
    }

    func updateNSView(_ splitView: NSSplitView, context: Context) {
        context.coordinator.owner = self
        splitView.isVertical = (axis == .horizontal)

        let currentIDs = Set(panes.map(\.id))
        for (paneID, session) in context.coordinator.sessions where !currentIDs.contains(paneID) {
            session.terminate()
            context.coordinator.sessions.removeValue(forKey: paneID)
            context.coordinator.paneIDBySession.removeValue(forKey: ObjectIdentifier(session))
        }

        for pane in panes where context.coordinator.sessions[pane.id] == nil {
            let session = PTYSession(frame: .zero)
            session.processDelegate = context.coordinator
            session.connect(to: pane.host) { jumpID in
                store.hosts.first { $0.id == jumpID }
            }
            context.coordinator.sessions[pane.id] = session
            context.coordinator.paneIDBySession[ObjectIdentifier(session)] = pane.id
        }

        let orderedSessions: [NSView] = panes.compactMap { context.coordinator.sessions[$0.id] }
        splitView.subviews = orderedSessions
        splitView.adjustSubviews()
    }

    final class Coordinator: NSObject, LocalProcessTerminalViewDelegate {
        var owner: SplitPaneView?
        var sessions: [UUID: PTYSession] = [:]
        var paneIDBySession: [ObjectIdentifier: UUID] = [:]

        func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}

        func setTerminalTitle(source: LocalProcessTerminalView, title: String) {}

        func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
            guard let session = source as? PTYSession,
                  let paneID = paneIDBySession[ObjectIdentifier(session)],
                  let raw = directory,
                  let path = OSC7.parsePath(from: raw) else { return }
            let callback = owner?.onCwdChange
            DispatchQueue.main.async {
                callback?(paneID, path)
            }
        }

        func processTerminated(source: TerminalView, exitCode: Int32?) {
            guard let session = source as? PTYSession,
                  let paneID = paneIDBySession[ObjectIdentifier(session)] else { return }
            let callback = owner?.onPaneExit
            DispatchQueue.main.async {
                callback?(paneID)
            }
        }
    }
}
