import Foundation
import AppKit
import Observation

/// Starts/stops background `ssh -N -L/-R/-D` processes for saved
/// TunnelRules — no pty needed, same reasoning as PTYSession: reuse real
/// ssh's port-forwarding rather than reimplementing it (e.g. via Citadel's
/// createDirectTCPIPChannel). Lives for the app's lifetime (owned by
/// MainWindowView), independent of any particular terminal tab or the SFTP
/// pane, so a tunnel keeps running even if you close the tab you opened it
/// from — matches MobaXterm's tunnel manager behavior.
@MainActor
@Observable
final class TunnelManager {
    private(set) var runningTunnelIDs: Set<UUID> = []
    private(set) var lastError: String?
    private var processes: [UUID: Process] = [:]

    init() {
        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.stopAll()
        }
    }

    func isRunning(_ tunnelID: UUID) -> Bool {
        runningTunnelIDs.contains(tunnelID)
    }

    func start(_ tunnel: TunnelRule, on host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) {
        guard !runningTunnelIDs.contains(tunnel.id) else { return }
        lastError = nil

        let executable: String
        let args: [String]
        do {
            (executable, args) = try TunnelArgvBuilder.build(for: tunnel, host: host, resolveJumpHost: resolveJumpHost)
        } catch {
            lastError = "Cannot start tunnel: \((error as? LocalizedError)?.errorDescription ?? "\(error)")"
            return
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = args
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice

        let tunnelID = tunnel.id
        process.terminationHandler = { [weak self] _ in
            Task { @MainActor in
                self?.runningTunnelIDs.remove(tunnelID)
                self?.processes.removeValue(forKey: tunnelID)
            }
        }

        do {
            try process.run()
            processes[tunnel.id] = process
            runningTunnelIDs.insert(tunnel.id)
        } catch {
            lastError = "Could not start tunnel “\(tunnel.name.isEmpty ? tunnel.kind.rawValue : tunnel.name)”: \(error.localizedDescription)"
        }
    }

    func stop(_ tunnelID: UUID) {
        guard let process = processes[tunnelID] else { return }
        process.terminate()
        processes.removeValue(forKey: tunnelID)
        runningTunnelIDs.remove(tunnelID)
    }

    func stopAll() {
        for process in processes.values {
            process.terminate()
        }
        processes.removeAll()
        runningTunnelIDs.removeAll()
    }
}
