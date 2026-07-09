import SwiftUI

/// Sessions | SFTP browser | Terminal — the MobaXterm-style three-column shell.
/// Double-clicking a host in the sidebar connects both the terminal and the
/// SFTP pane together, matching MobaXterm's feel.
struct MainWindowView: View {
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var store = SessionStore()
    @State private var tabs: [TerminalTab] = []
    @State private var selectedTabID: UUID?
    @State private var sftpConnection = SFTPConnection()
    @State private var tunnelManager = TunnelManager()

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
    }

    private var selectedTabTitle: String {
        tabs.first { $0.id == selectedTabID }?.title ?? "Terminal"
    }

    /// Reuses an existing tab for this host if one's already open (matches
    /// MobaXterm's double-click-to-focus behavior); otherwise opens a new
    /// tab and (re)connects the SFTP pane to the same host.
    private func connect(to host: Host) {
        if let existing = tabs.first(where: { $0.panes.contains { $0.host.id == host.id } }) {
            selectedTabID = existing.id
        } else {
            let tab = TerminalTab(title: host.name, panes: [TerminalTabPane(host: host)])
            tabs.append(tab)
            selectedTabID = tab.id
        }
        Task { await sftpConnection.connect(to: host) }
    }
}

#Preview {
    MainWindowView()
}
