import SwiftUI

/// Sessions | SFTP browser | Terminal — the MobaXterm-style three-column shell.
/// The session sidebar and terminal core are real; the SFTP pane is still a
/// placeholder until that milestone lands.
struct MainWindowView: View {
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var store = SessionStore()
    @State private var tabs: [TerminalTab] = []
    @State private var selectedTabID: UUID?

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SessionSidebarView(store: store, onConnect: connect)
                .navigationTitle("Sessions")
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 340)
        } content: {
            SFTPBrowserPlaceholder()
                .navigationSplitViewColumnWidth(min: 260, ideal: 360, max: 600)
        } detail: {
            TerminalContainerView(store: store, tabs: $tabs, selectedTabID: $selectedTabID)
                .navigationTitle(selectedTabTitle)
        }
        .navigationSplitViewStyle(.balanced)
    }

    private var selectedTabTitle: String {
        tabs.first { $0.id == selectedTabID }?.title ?? "Terminal"
    }

    /// Reuses an existing tab for this host if one's already open (matches
    /// MobaXterm's double-click-to-focus behavior); otherwise opens a new one.
    private func connect(to host: Host) {
        if let existing = tabs.first(where: { $0.panes.contains { $0.host.id == host.id } }) {
            selectedTabID = existing.id
            return
        }
        let tab = TerminalTab(title: host.name, panes: [TerminalTabPane(host: host)])
        tabs.append(tab)
        selectedTabID = tab.id
    }
}

private struct SFTPBrowserPlaceholder: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("SFTP browser")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .navigationTitle("Files")
    }
}

#Preview {
    MainWindowView()
}
