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
    /// Open tabs + selection. App-scoped (see WorkspaceModel) so closing the
    /// window to the menu bar doesn't orphan the live ssh sessions.
    let workspace: WorkspaceModel

    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    private var tabs: [WorkspaceTab] { workspace.tabs }
    private var selectedTabID: UUID? { workspace.selectedTabID }

    var body: some View {
        @Bindable var workspace = workspace
        return NavigationSplitView(columnVisibility: $columnVisibility) {
            SessionSidebarView(store: store, tunnelManager: tunnelManager,
                               connectedHostIDs: connectedHostIDs, onConnect: connect)
                .navigationTitle("Sessions")
                .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 340)
        } content: {
            // Shows the selected terminal's OWN long-lived SFTP session, so
            // switching tabs swaps the view rather than reconnecting.
            SFTPBrowserView(connection: activeSFTPConnection)
                .id(selectedHost?.id)
                .navigationSplitViewColumnWidth(min: 260, ideal: 360, max: 600)
                .navigationTitle("Files")
        } detail: {
            TerminalContainerView(
                store: store,
                tabs: $workspace.tabs,
                selectedTabID: $workspace.selectedTabID,
                onCwdChange: { host, path in
                    if let connection = sftpRegistry.existing(for: host.id) {
                        Task { await connection.terminalDidReportCwd(host: host, path: path) }
                    }
                },
                onReady: { host in
                    // Silent signal that the shared SSH connection is up — lets
                    // SFTP auto-(re)connect if its initial attempt lost the race
                    // with the terminal's authentication (no shell hook needed).
                    if let connection = sftpRegistry.existing(for: host.id) {
                        Task { await connection.terminalDidBecomeReady(host: host) }
                    }
                }
            )
            .navigationTitle(selectedTabTitle)
        }
        .navigationSplitViewStyle(.balanced)
        .background(WindowConfigurator())
        // An SFTP session rides its terminal's authenticated connection, so drop
        // any whose host no longer has an open terminal.
        .onChange(of: terminalHostIDs) { _, ids in
            Task { await sftpRegistry.pruneSessions(keepingHostIDs: ids) }
        }
        // Selecting a tab shows that host's session, connecting it only the
        // first time (or if it was disconnected).
        .onChange(of: selectedTabID) { _, _ in
            activateSFTPForSelectedTab()
        }
    }

    private var sftpRegistry: SFTPSessionRegistry { SFTPSessionRegistry.shared }

    /// Host of the selected tab, when it's a terminal (SFTP is SSH-only).
    private var selectedHost: Host? {
        guard let tab = tabs.first(where: { $0.id == selectedTabID }),
              case .terminal(let terminalTab) = tab else { return nil }
        return terminalTab.panes.first?.host
    }

    private var activeSFTPConnection: SFTPConnection? {
        guard let host = selectedHost else { return nil }
        return sftpRegistry.existing(for: host.id)
    }

    private func activateSFTPForSelectedTab() {
        guard let host = selectedHost else { return }
        sftpRegistry.activate(host: host) { jumpID in
            store.hosts.first { $0.id == jumpID }
        }
    }

    /// Hosts with an open terminal tab (SFTP pairs with these).
    private var terminalHostIDs: Set<UUID> {
        Set(tabs.flatMap { tab -> [UUID] in
            if case .terminal(let t) = tab { return t.panes.map(\.host.id) }
            return []
        })
    }

    /// Every connected host (terminal + VNC) — drives the sidebar status dot.
    private var connectedHostIDs: Set<UUID> {
        var ids = terminalHostIDs
        for tab in tabs {
            if case .vnc(let v) = tab { ids.insert(v.host.id) }
        }
        return ids
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
            workspace.selectedTabID = existingTerminalTab.id
        } else {
            let tab = TerminalTab(title: host.name, panes: [TerminalTabPane(host: host)])
            workspace.tabs.append(.terminal(tab))
            workspace.selectedTabID = tab.id
        }
        sftpRegistry.activate(host: host) { jumpID in
            store.hosts.first { $0.id == jumpID }
        }
    }

    private func connectVNC(_ host: Host) {
        let existingVNCTab = tabs.first { tab in
            if case .vnc(let vncTab) = tab {
                return vncTab.host.id == host.id
            }
            return false
        }

        if let existingVNCTab {
            workspace.selectedTabID = existingVNCTab.id
        } else {
            let tab = VNCTab(host: host, title: host.name)
            workspace.tabs.append(.vnc(tab))
            workspace.selectedTabID = tab.id
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
    MainWindowView(store: SessionStore(), tunnelManager: TunnelManager(),
                   workspace: WorkspaceModel())
}
