import SwiftUI

@main
struct BHTerminalApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    // Owned at App scope so they outlive the window: closing to the menu bar
    // keeps saved sessions, running tunnels, open session tabs and the SFTP
    // connection intact (and stops live ssh sessions from being orphaned when
    // the window's views are destroyed).
    @State private var store = SessionStore()
    @State private var tunnelManager = TunnelManager()
    @State private var workspace = WorkspaceModel()
    @State private var sftpConnection = SFTPConnection()

    var body: some Scene {
        // A single main window (id "main") — closing it drops the app to the
        // menu bar; "Open BHTerminal" re-opens this exact window.
        Window("BHTerminal", id: "main") {
            MainWindowView(store: store, tunnelManager: tunnelManager,
                           workspace: workspace, sftpConnection: sftpConnection)
        }
        .defaultSize(width: 1280, height: 800)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(after: .newItem) {
                Button("Import Sessions…") {
                    SessionImportUI.run(into: store)
                }
                .keyboardShortcut("i", modifiers: [.command, .shift])
            }
        }

        Settings {
            PreferencesView()
        }

        MenuBarExtra("BHTerminal", systemImage: "terminal") {
            MenuBarContent()
        }
    }
}

/// Contents of the menu-bar icon's menu. Kept as its own view so it can pull
/// `openWindow` from the environment.
private struct MenuBarContent: View {
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Open BHTerminal") {
            AppActivation.foreground()
            openWindow(id: "main")
        }
        .keyboardShortcut("o")

        Divider()

        Button("Quit BHTerminal") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
