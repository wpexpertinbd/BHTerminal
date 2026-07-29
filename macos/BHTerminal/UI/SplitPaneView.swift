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

/// SwiftTerm's TerminalView has no built-in content padding — text renders
/// flush against the view's edges, which reads as cramped. Wraps a
/// PTYSession in a plain NSView with the session inset by a fixed margin
/// via Auto Layout (the container itself stays frame-based/autoresizing so
/// NSSplitView's own layout is untouched; only the session inside opts
/// into constraints). Background matches the session's own resolved theme
/// color so the margin doesn't read as a mismatched border.
final class PaddedTerminalContainer: NSView {
    static let padding: CGFloat = 8

    /// Held so the margin can keep matching the terminal's own background. A
    /// fixed snapshot of the color went wrong the moment the two could differ:
    /// taken before the theme was applied it left a black frame around a light
    /// terminal, and it never followed a theme change afterwards.
    private weak var session: PTYSession?

    override func layout() {
        super.layout()
        if let session {
            layer?.backgroundColor = session.nativeBackgroundColor.cgColor
        }
    }

    init(session: PTYSession) {
        self.session = session
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = session.nativeBackgroundColor.cgColor

        session.translatesAutoresizingMaskIntoConstraints = false
        addSubview(session)
        NSLayoutConstraint.activate([
            session.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.padding),
            session.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Self.padding),
            session.topAnchor.constraint(equalTo: topAnchor, constant: Self.padding),
            session.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -Self.padding)
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

/// Wraps an NSSplitView whose arranged subviews are PTYSession instances,
/// one per TerminalTabPane.
///
/// The sessions themselves are owned by TerminalSessionRegistry (app
/// lifetime), NOT by this view — SwiftUI tears NSViewRepresentables down
/// whenever it likes, and this one used to kill the tab's ssh processes when
/// it did. This view only borrows the live sessions and arranges them.
struct SplitPaneView: NSViewRepresentable {
    let panes: [TerminalTabPane]
    let axis: Axis
    let store: SessionStore
    var broadcastEnabled: Bool = false
    var commandDispatch: PaneCommandDispatch?
    /// False for background tabs — they stay connected but hidden (and can't
    /// steal keyboard focus from the visible tab).
    var isActive: Bool = true
    /// Changes when a pane reconnects, so this view picks up the new session's
    /// container view.
    var reconnectToken: Int = 0
    var onPaneExit: (UUID, Int32?) -> Void = { _, _ in }
    var onCwdChange: (UUID, String) -> Void = { _, _ in }
    /// Fires (once per pane) when its login settles into a shell prompt — the
    /// silent signal the SFTP pane uses to auto-connect.
    var onReady: (UUID) -> Void = { _ in }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    /// Deliberately does NOT terminate anything: the registry owns the ssh
    /// processes so that switching tabs (which tears this view down) leaves
    /// every session connected. Panes/tabs are terminated explicitly by
    /// TerminalContainerView when the user actually closes them.
    static func dismantleNSView(_ nsView: NSSplitView, coordinator: Coordinator) {
        coordinator.removeKeyMonitor()
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

        let registry = TerminalSessionRegistry.shared
        let coordinator = context.coordinator
        coordinator.sessions.removeAll()
        coordinator.paneIDBySession.removeAll()

        var orderedContainers: [NSView] = []
        for pane in panes {
            let paneID = pane.id
            let session = registry.session(for: paneID, host: pane.host) { jumpID in
                store.hosts.first { $0.id == jumpID }
            }
            // Re-point the callbacks at the current coordinator: this view (and
            // its coordinator) can be rebuilt while the session lives on.
            session.processDelegate = coordinator
            session.onReady = { [weak coordinator] in
                coordinator?.owner?.onReady(paneID)
            }
            coordinator.sessions[paneID] = session
            coordinator.paneIDBySession[ObjectIdentifier(session)] = paneID
            if let container = registry.container(for: paneID) {
                orderedContainers.append(container)
            }
        }

        // Only re-arrange when the set actually changed — reassigning subviews
        // on every SwiftUI re-render would thrash NSSplitView's layout.
        if splitView.arrangedSubviews != orderedContainers {
            splitView.subviews = orderedContainers
            splitView.adjustSubviews()
        }

        // Background tabs stay live but hidden; hiding also makes AppKit give
        // up first-responder status so typing can't leak into a hidden tab.
        splitView.isHidden = !isActive
        if isActive && !coordinator.wasActive {
            let firstSession = panes.first.flatMap { coordinator.sessions[$0.id] }
            DispatchQueue.main.async {
                if let firstSession, let window = firstSession.window {
                    window.makeFirstResponder(firstSession)
                }
            }
        }
        coordinator.wasActive = isActive

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
        /// Borrowed from TerminalSessionRegistry (which owns them) — refreshed
        /// on every updateNSView. Never terminated from here.
        var sessions: [UUID: PTYSession] = [:]
        var paneIDBySession: [ObjectIdentifier: UUID] = [:]
        var lastCommandID: UUID?
        var wasActive = false
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

        func removeKeyMonitor() {
            if let keyMonitor {
                NSEvent.removeMonitor(keyMonitor)
                self.keyMonitor = nil
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
                callback?(paneID, exitCode)
            }
        }
    }
}
