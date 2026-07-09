import SwiftUI
import AppKit
import RoyalVNCKit

/// Bridges a VNCSession's stable container view into SwiftUI. The session
/// (and its underlying connection) is created once per Coordinator and
/// only torn down via dismantleNSView — same discipline as SplitPaneView,
/// so switching tabs away from a VNC session actually disconnects it
/// instead of leaking a live socket.
struct VNCPaneView: NSViewRepresentable {
    let host: Host
    var onStateChange: (VNCConnection.Status, Error?) -> Void = { _, _ in }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.session?.disconnect()
    }

    func makeNSView(context: Context) -> NSView {
        let session = VNCSession()
        session.onStateChange = { [weak coordinator = context.coordinator] status, error in
            coordinator?.owner?.onStateChange(status, error)
        }
        session.connect(to: host)
        context.coordinator.session = session
        context.coordinator.owner = self
        return session.containerView
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.owner = self
    }

    final class Coordinator {
        var session: VNCSession?
        var owner: VNCPaneView?
    }
}

/// The actual VNC tab content: the framebuffer view plus a connecting/
/// error overlay, since RoyalVNCKit shows nothing at all until the first
/// framebuffer arrives — a bare blank view during that window reads as
/// broken rather than "connecting".
struct VNCTabView: View {
    let host: Host

    @State private var status: VNCConnection.Status = .connecting
    @State private var error: Error?

    var body: some View {
        ZStack {
            VNCPaneView(host: host) { newStatus, newError in
                status = newStatus
                error = newError
            }

            if status != .connected {
                VStack(spacing: 8) {
                    if let error {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.system(size: 32))
                            .foregroundStyle(.orange)
                        Text(error.localizedDescription)
                            .font(.caption)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 20)
                    } else {
                        ProgressView()
                        Text("Connecting to \(host.hostname)…")
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.black.opacity(0.85))
            }
        }
    }
}
