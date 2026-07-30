import SwiftUI

/// The MobaXterm-style session tree: folders + hosts, add/edit/delete,
/// double-click (or Return) to connect. `onConnect` is a no-op until the
/// terminal core exists to actually act on it.
struct SessionSidebarView: View {
    var store: SessionStore
    var tunnelManager: TunnelManager
    /// Hosts with a live connection (open terminal/VNC tab) — shown as a green
    /// status dot; idle hosts keep their colour tag (or grey).
    var connectedHostIDs: Set<UUID> = []
    var onConnect: (Host) -> Void = { _ in }

    @State private var searchText = ""
    @State private var selection: UUID?
    @State private var sheetTarget: SheetTarget?
    @State private var pendingDeletion: DeletionTarget?

    private enum SheetTarget: Identifiable {
        case newHost(folderID: UUID?)
        case editHost(Host)
        case newFolder(parentID: UUID?)
        case renameFolder(HostFolder)
        case tunnels(Host)

        var id: String {
            switch self {
            case .newHost(let id): return "newHost-\(id?.uuidString ?? "root")"
            case .editHost(let host): return "editHost-\(host.id)"
            case .newFolder(let id): return "newFolder-\(id?.uuidString ?? "root")"
            case .renameFolder(let folder): return "renameFolder-\(folder.id)"
            case .tunnels(let host): return "tunnels-\(host.id)"
            }
        }
    }

    private enum DeletionTarget: Identifiable {
        case host(Host)
        case folder(HostFolder)

        var id: UUID {
            switch self {
            case .host(let host): return host.id
            case .folder(let folder): return folder.id
            }
        }

        var name: String {
            switch self {
            case .host(let host): return host.name
            case .folder(let folder): return folder.name
            }
        }
    }

    private var filteredHosts: [Host] {
        guard !searchText.isEmpty else { return [] }
        let needle = searchText.lowercased()
        return store.hosts.filter {
            $0.name.lowercased().contains(needle)
                || $0.hostname.lowercased().contains(needle)
                || $0.username.lowercased().contains(needle)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            List(selection: $selection) {
                if searchText.isEmpty {
                    OutlineGroup(store.tree, children: \.children) { item in
                        row(for: item)
                    }
                } else {
                    ForEach(filteredHosts) { host in
                        hostRow(host)
                    }
                }
            }
            .listStyle(.sidebar)
            .searchable(text: $searchText, placement: .sidebar, prompt: "Quick connect / filter")
            // NOTE: deliberately NOT using .contextMenu(forSelectionType:) here.
            // Layering that on a List whose rows already have their own
            // .contextMenu — and whose content is an OutlineGroup — left the
            // sidebar's SwiftUI updates wedged after a few edits: sheets, the
            // delete confirmation and even folder expand/collapse silently
            // stopped responding (only relaunching fixed it). Double-click to
            // connect is handled per-row instead (see hostRow).
        }
        .toolbar {
            ToolbarItem {
                Menu {
                    Button("New Host…", systemImage: "plus") {
                        present(.newHost(folderID: nil))
                    }
                    Button("New Folder…", systemImage: "folder.badge.plus") {
                        present(.newFolder(parentID: nil))
                    }
                    Divider()
                    Button("Import Sessions…", systemImage: "square.and.arrow.down") {
                        SessionImportUI.run(into: store)
                    }
                } label: {
                    Image(systemName: "plus")
                }
            }
        }
        .sheet(item: $sheetTarget, onDismiss: { sheetTarget = nil }) { target in
            sheetContent(for: target)
        }
        .confirmationDialog(
            deletionMessage,
            isPresented: Binding(get: { pendingDeletion != nil }, set: { if !$0 { pendingDeletion = nil } }),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) { performPendingDeletion() }
            Button("Cancel", role: .cancel) {}
        }
    }

    // MARK: - Presentation (self-healing)

    /// Every sheet goes through here instead of assigning `sheetTarget` directly.
    ///
    /// `.sheet(item:)` ignores a new value while it believes one is presented, and
    /// dismissal is driven by the child's `@Environment(\.dismiss)` — so a single
    /// lost dismissal left `sheetTarget` set forever and silently killed EVERY
    /// later sheet AND the delete confirmation (the reported "sidebar menu stops
    /// responding after adding a few hosts"; only relaunching, which resets
    /// @State, brought it back).
    ///
    /// The recovery is safe by construction: a sheet is modal to its window, so
    /// if the user was able to trigger this at all, nothing is actually on
    /// screen and any leftover value is stale. Clear it, then present on the
    /// next tick so SwiftUI sees a real nil→value transition.
    private func present(_ target: SheetTarget) {
        guard sheetTarget != nil || pendingDeletion != nil else {
            sheetTarget = target
            return
        }
        sheetTarget = nil
        pendingDeletion = nil
        DispatchQueue.main.async { sheetTarget = target }
    }

    /// Same reasoning for the delete confirmation.
    private func requestDeletion(_ target: DeletionTarget) {
        guard sheetTarget != nil || pendingDeletion != nil else {
            pendingDeletion = target
            return
        }
        sheetTarget = nil
        pendingDeletion = nil
        DispatchQueue.main.async { pendingDeletion = target }
    }

    private var deletionMessage: String {
        switch pendingDeletion {
        case .host(let host): return "Delete “\(host.name)”?"
        case .folder(let folder): return "Delete “\(folder.name)” and everything inside it?"
        case nil: return ""
        }
    }

    // MARK: - Rows

    @ViewBuilder
    private func row(for item: SessionTreeItem) -> some View {
        switch item {
        case .folder(let folder, _):
            folderRow(folder)
        case .host(let host):
            hostRow(host)
        }
    }

    private func folderRow(_ folder: HostFolder) -> some View {
        Label(folder.name, systemImage: "folder")
            .contextMenu {
                Button("New Host Here…", systemImage: "plus") {
                    present(.newHost(folderID: folder.id))
                }
                Button("New Subfolder…", systemImage: "folder.badge.plus") {
                    present(.newFolder(parentID: folder.id))
                }
                Divider()
                Button("Rename…", systemImage: "pencil") {
                    present(.renameFolder(folder))
                }
                Button("Delete", systemImage: "trash", role: .destructive) {
                    requestDeletion(.folder(folder))
                }
            }
    }

    private func hostRow(_ host: Host) -> some View {
        let connected = connectedHostIDs.contains(host.id)
        return HStack(spacing: 8) {
            Circle()
                .fill(connected ? Color.green : colorForTag(host.colorTag))
                .frame(width: 8, height: 8)
                .help(connected ? "Connected" : "Not connected")
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    if host.connectionType == .vnc {
                        Image(systemName: "display")
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                    Text(host.name)
                }
                Text(subtitle(for: host))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .tag(host.id)
        .contentShape(Rectangle())
        // Double-click connects; the single-click case sets the selection
        // explicitly, because a tap gesture on the row would otherwise swallow
        // the click the List uses to highlight it. Order matters: the count: 2
        // gesture has to come first to get a chance to match.
        .onTapGesture(count: 2) { onConnect(host) }
        .onTapGesture { selection = host.id }
        .contextMenu {
            Button("Connect", systemImage: "bolt.fill") { onConnect(host) }
            Divider()
            Button("Edit…", systemImage: "pencil") { present(.editHost(host)) }
            Button("Duplicate", systemImage: "plus.square.on.square") { duplicate(host) }
            Button("Tunnels…", systemImage: "arrow.left.arrow.right") { present(.tunnels(host)) }
            Divider()
            Button("Delete", systemImage: "trash", role: .destructive) {
                requestDeletion(.host(host))
            }
        }
    }

    private func subtitle(for host: Host) -> String {
        switch host.connectionType {
        case .ssh:
            return "\(host.username)@\(host.hostname):\(host.port)"
        case .vnc:
            return "\(host.hostname):\(host.port)"
        }
    }

    private func colorForTag(_ tag: String?) -> Color {
        switch tag {
        case "red": return .red
        case "orange": return .orange
        case "yellow": return .yellow
        case "green": return .green
        case "blue": return .blue
        case "purple": return .purple
        default: return .secondary
        }
    }

    // MARK: - Sheets

    @ViewBuilder
    private func sheetContent(for target: SheetTarget) -> some View {
        switch target {
        case .newHost(let folderID):
            HostEditorView(store: store, folderID: folderID)
        case .editHost(let host):
            HostEditorView(store: store, editingHost: host)
        case .newFolder(let parentID):
            TextPromptSheet(title: "New Folder", placeholder: "Folder name") { name in
                store.addFolder(HostFolder(name: name, parentFolderID: parentID))
            }
        case .tunnels(let host):
            TunnelManagerView(store: store, manager: tunnelManager, host: host)
        case .renameFolder(let folder):
            TextPromptSheet(title: "Rename Folder", initialValue: folder.name, placeholder: "Folder name") { name in
                var updated = folder
                updated.name = name
                store.updateFolder(updated)
            }
        }
    }

    // MARK: - Actions

    private func duplicate(_ host: Host) {
        var copy = host
        copy.id = UUID()
        copy.name = host.name + " Copy"
        let saved = store.addHost(copy)

        if host.authMethod == .password, let secret = try? KeychainService.read(account: host.keychainAccount) {
            try? KeychainService.save(account: saved.keychainAccount, secret: secret)
        }
        if case .privateKey = host.authMethod, let secret = try? KeychainService.read(account: host.passphraseAccount) {
            try? KeychainService.save(account: saved.passphraseAccount, secret: secret)
        }
    }

    private func performPendingDeletion() {
        switch pendingDeletion {
        case .host(let host): store.deleteHost(host.id)
        case .folder(let folder): store.deleteFolder(folder.id)
        case nil: break
        }
        pendingDeletion = nil
    }
}
