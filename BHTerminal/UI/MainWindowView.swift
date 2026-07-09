import SwiftUI

/// Sessions | SFTP browser | Terminal — the MobaXterm-style three-column shell.
/// Double-clicking a host in the sidebar connects both the terminal and the
/// SFTP pane together, matching MobaXterm's feel.
struct MainWindowView: View {
    /// store + tunnelManager are owned by the App (process lifetime), not by
    /// this view, so closing the window to the menu bar doesn't tear down
    /// saved sessions or kill running tunnels.
    let store: SessionStore
    let tunnelManager: TunnelManager

    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var tabs: [WorkspaceTab] = []
    @State private var selectedTabID: UUID?
    @State private var sftpConnection = SFTPConnection()

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SessionSidebarView(store: store, tunnelManager: tunnelManager, onConnect: connect)
                .navigationTitle("Sessions")
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 340)
        } content: {
            SFTPBrowserView(connection: sftpConnection)
                .navigationSplitViewColumnWidth(min: 260, ideal: 360, max: 600)
                .navigationTitle("Files")
        } detail: {
            TerminalContainerView(
                store: store,
                tabs: $tabs,
                selectedTabID: $selectedTabID,
                onCwdChange: { host, path in
                    guard host.id == sftpConnection.connectedHost?.id else { return }
                    Task { await sftpConnection.navigateToAbsolutePath(path) }
                }
            )
            .navigationTitle(selectedTabTitle)
        }
        .navigationSplitViewStyle(.balanced)
        .background(WindowConfigurator())
    }

    private var selectedTabTitle: String {
        tabs.first { $0.id == selectedTabID }?.title ?? "Terminal"
    }

    /// Reuses an existing tab for this host if one's already open (matches
    /// MobaXterm's double-click-to-focus behavior); otherwise opens a new
    /// one. VNC hosts get a single-view tab with no SFTP pairing — SFTP is
    /// an SSH-only concept.
    private func connect(to host: Host) {
        switch host.connectionType {
        case .ssh:
            connectSSH(host)
        case .vnc:
            connectVNC(host)
        }
    }

    private func connectSSH(_ host: Host) {
        let existingTerminalTab = tabs.first { tab in
            if case .terminal(let terminalTab) = tab {
                return terminalTab.panes.contains { $0.host.id == host.id }
            }
            return false
        }

        if let existingTerminalTab {
            selectedTabID = existingTerminalTab.id
        } else {
            let tab = TerminalTab(title: host.name, panes: [TerminalTabPane(host: host)])
            tabs.append(.terminal(tab))
            selectedTabID = tab.id
        }
        Task { await sftpConnection.connect(to: host) }
    }

    private func connectVNC(_ host: Host) {
        let existingVNCTab = tabs.first { tab in
            if case .vnc(let vncTab) = tab {
                return vncTab.host.id == host.id
            }
            return false
        }

        if let existingVNCTab {
            selectedTabID = existingVNCTab.id
        } else {
            let tab = VNCTab(host: host, title: host.name)
            tabs.append(.vnc(tab))
            selectedTabID = tab.id
        }
    }
}

/// Tags the host NSWindow so AppDelegate can recognise the main window
/// closing (→ drop to the menu bar / background) without guessing.
private struct WindowConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            view.window?.identifier = NSUserInterfaceItemIdentifier(BHTerminalWindow.mainID)
        }
        return view
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

enum BHTerminalWindow {
    static let mainID = "bhterminal-main"
}

#Preview {
    MainWindowView(store: SessionStore(), tunnelManager: TunnelManager())
}
