import AppKit
import RoyalVNCKit

/// Wraps a RoyalVNCKit VNCConnection + the container that hosts its
/// framebuffer view. VNCCAFramebufferView needs the actual VNCFramebuffer
/// at init time (only available once connected), so this holds a stable
/// empty container NSView from the start and swaps the real framebuffer
/// view in once didCreateFramebuffer fires (and again on resize — no
/// resize-in-place API is exposed on VNCCAFramebufferView, so recreating
/// is the safe path, just marginally less efficient than an in-place
/// resize would be).
final class VNCSession: NSObject, VNCConnectionDelegate {
    let containerView = NSView()
    private(set) var host: Host?
    private var connection: VNCConnection?
    private var framebufferView: VNCCAFramebufferView?

    var onStateChange: ((VNCConnection.Status, Error?) -> Void)?

    func connect(to host: Host) {
        self.host = host

        let settings = VNCConnection.Settings(
            isDebugLoggingEnabled: false,
            hostname: host.hostname,
            port: UInt16(clamping: host.port),
            isShared: true,
            isScalingEnabled: true,
            useDisplayLink: true,
            // .forwardAllKeyboardShortcutsAndHotKeys needs Accessibility
            // permission granted to the app; this mode doesn't.
            inputMode: .forwardKeyboardShortcutsIfNotInUseLocally,
            isClipboardRedirectionEnabled: true,
            colorDepth: .depth24Bit,
            frameEncodings: VNCFrameEncodingType.defaultFrameEncodings
        )

        let connection = VNCConnection(settings: settings)
        connection.delegate = self
        self.connection = connection
        connection.connect()
    }

    func disconnect() {
        connection?.disconnect()
    }

    // MARK: - VNCConnectionDelegate

    func connection(_ connection: VNCConnection, stateDidChange connectionState: VNCConnection.ConnectionState) {
        let status = connectionState.status
        let error = connectionState.error
        DispatchQueue.main.async { [weak self] in
            self?.onStateChange?(status, error)
            if status == .disconnected {
                self?.framebufferView?.removeFromSuperview()
                self?.framebufferView = nil
            }
        }
    }

    func connection(_ connection: VNCConnection, credentialFor authenticationType: VNCAuthenticationType, completion: @escaping (VNCCredential?) -> Void) {
        guard let host else {
            completion(nil)
            return
        }
        let password = (try? KeychainService.read(account: host.keychainAccount)) ?? ""

        if authenticationType.requiresUsername {
            completion(VNCUsernamePasswordCredential(username: host.username, password: password))
        } else if authenticationType.requiresPassword {
            completion(VNCPasswordCredential(password: password))
        } else {
            completion(nil)
        }
    }

    func connection(_ connection: VNCConnection, didCreateFramebuffer framebuffer: VNCFramebuffer) {
        DispatchQueue.main.async { [weak self] in
            self?.installFramebufferView(framebuffer: framebuffer, connection: connection)
        }
    }

    func connection(_ connection: VNCConnection, didResizeFramebuffer framebuffer: VNCFramebuffer) {
        DispatchQueue.main.async { [weak self] in
            self?.installFramebufferView(framebuffer: framebuffer, connection: connection)
        }
    }

    func connection(_ connection: VNCConnection, didUpdateFramebuffer framebuffer: VNCFramebuffer, x: UInt16, y: UInt16, width: UInt16, height: UInt16) {
        // Consumed internally by VNCCAFramebufferView once installed — it
        // re-assigns connection.delegate to itself and forwards what it
        // doesn't handle back to us (see RoyalVNCKit's USAGE.md).
    }

    func connection(_ connection: VNCConnection, didUpdateCursor cursor: VNCCursor) {
        // Same as above — handled internally by VNCCAFramebufferView.
    }

    private func installFramebufferView(framebuffer: VNCFramebuffer, connection: VNCConnection) {
        framebufferView?.removeFromSuperview()

        let view = VNCCAFramebufferView(
            frame: containerView.bounds,
            framebuffer: framebuffer,
            connection: connection,
            connectionDelegate: self
        )
        view.autoresizingMask = [.width, .height]
        containerView.addSubview(view)
        framebufferView = view
    }
}
