import AppKit

/// Drives the "close to menu bar" behaviour: clicking the window's red close
/// button doesn't quit — the app keeps running (menu bar icon stays, tunnels
/// keep forwarding) and the Dock icon drops away so it's a true background
/// app. Re-opening from the menu bar restores the Dock icon + window.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(windowWillClose(_:)),
            name: NSWindow.willCloseNotification,
            object: nil
        )
    }

    /// Keep running after the main window closes (menu bar app).
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Close every live session on quit. The sessions are owned by
    /// TerminalSessionRegistry (app lifetime), so nothing else tears them down —
    /// and this also shuts down their shared ssh ControlMasters, so the next
    /// launch never inherits a stale one.
    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated {
            TerminalSessionRegistry.shared.terminateAll()
        }
    }

    /// Clicking the Dock icon (when it's showing) or re-launching brings the
    /// window back and restores regular (Dock-visible) mode.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows {
            AppActivation.foreground()
        }
        return true
    }

    @objc private func windowWillClose(_ note: Notification) {
        guard let window = note.object as? NSWindow,
              window.identifier?.rawValue == BHTerminalWindow.mainID else { return }
        // Defer so the close finishes before we change activation policy.
        DispatchQueue.main.async {
            AppActivation.background()
        }
    }
}

/// Switches between a normal foreground app (Dock icon + menus) and a
/// background menu-bar-only app. Shared so both the AppDelegate and the
/// MenuBarExtra's "Open" action drive the same transitions.
enum AppActivation {
    static func background() {
        NSApp.setActivationPolicy(.accessory)
    }

    static func foreground() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }
}
