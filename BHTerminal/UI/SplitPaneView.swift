import SwiftUI
import AppKit
import SwiftTerm

/// A one-shot imperative command aimed at "the active pane" of a split
/// group — snippet dispatch and session-logging toggles both go through
/// this, since SplitPaneView's Coordinator (and the PTYSession instances it
/// owns) aren't otherwise reachable from TerminalContainerView. Wrapped
/// with a unique id so the same command (e.g. re-sending the same snippet
/// twice in a row) is still recognized as a new event by updateNSView.
enum PaneCommand: Equatable {
    case sendText(String)
    case startLogging(URL)
    case stopLogging
}

struct PaneCommandDispatch: Equatable {
    let id = UUID()
    let command: PaneCommand
}

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
    var broadcastEnabled: Bool = false
    var commandDispatch: PaneCommandDispatch?
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
        context.coordinator.installKeyMonitor()
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

        if let dispatch = commandDispatch, context.coordinator.lastCommandID != dispatch.id {
            context.coordinator.lastCommandID = dispatch.id
            if let target = context.coordinator.targetSession() {
                switch dispatch.command {
                case .sendText(let text): target.sendCommand(text)
                case .startLogging(let url): try? target.startLogging(to: url)
                case .stopLogging: target.stopLogging()
                }
            }
        }
    }

    final class Coordinator: NSObject, LocalProcessTerminalViewDelegate {
        var owner: SplitPaneView?
        var sessions: [UUID: PTYSession] = [:]
        var paneIDBySession: [ObjectIdentifier: UUID] = [:]
        var lastCommandID: UUID?
        private var keyMonitor: Any?

        /// The pane that should receive a dispatched snippet/logging
        /// command: whichever session is first responder, falling back to
        /// the first pane when nothing in this split group has focus.
        func targetSession() -> PTYSession? {
            if let responder = NSApp.keyWindow?.firstResponder as? PTYSession,
               sessions.values.contains(where: { $0 === responder }) {
                return responder
            }
            return sessions.values.first
        }

        /// Broadcast input can't be built by overriding PTYSession's
        /// keyDown (SwiftTerm declares it `public`, not `open` — not
        /// overridable outside the module). A local NSEvent monitor gets
        /// the same effect: when broadcast mode is on and a keyDown lands
        /// on one of this group's sessions, replay it (`keyDown` itself
        /// IS public, just not overridable) to every sibling pane.
        func installKeyMonitor() {
            guard keyMonitor == nil else { return }
            keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                self?.relayIfBroadcasting(event)
                return event
            }
        }

        private func relayIfBroadcasting(_ event: NSEvent) {
            guard owner?.broadcastEnabled == true,
                  let responder = NSApp.keyWindow?.firstResponder as? PTYSession,
                  let origin = sessions.values.first(where: { $0 === responder }) else { return }
            for session in sessions.values where session !== origin {
                session.keyDown(with: event)
            }
        }

        deinit {
            if let keyMonitor {
                NSEvent.removeMonitor(keyMonitor)
            }
        }

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
