import SwiftUI

/// The MobaXterm-style session tree: folders + hosts, add/edit/delete,
/// double-click (or Return) to connect. `onConnect` is a no-op until the
/// terminal core exists to actually act on it.
struct SessionSidebarView: View {
    var store: SessionStore
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

        var id: String {
            switch self {
            case .newHost(let id): return "newHost-\(id?.uuidString ?? "root")"
            case .editHost(let host): return "editHost-\(host.id)"
            case .newFolder(let id): return "newFolder-\(id?.uuidString ?? "root")"
            case .renameFolder(let folder): return "renameFolder-\(folder.id)"
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
        }
        .toolbar {
            ToolbarItem {
                Menu {
                    Button("New Host…", systemImage: "plus") {
                        sheetTarget = .newHost(folderID: nil)
                    }
                    Button("New Folder…", systemImage: "folder.badge.plus") {
                        sheetTarget = .newFolder(parentID: nil)
                    }
                } label: {
                    Image(systemName: "plus")
                }
            }
        }
        .sheet(item: $sheetTarget) { target in
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
                    sheetTarget = .newHost(folderID: folder.id)
                }
                Button("New Subfolder…", systemImage: "folder.badge.plus") {
                    sheetTarget = .newFolder(parentID: folder.id)
                }
                Divider()
                Button("Rename…", systemImage: "pencil") {
                    sheetTarget = .renameFolder(folder)
                }
                Button("Delete", systemImage: "trash", role: .destructive) {
                    pendingDeletion = .folder(folder)
                }
            }
    }

    private func hostRow(_ host: Host) -> some View {
        HStack(spacing: 8) {
            Circle()
                .fill(colorForTag(host.colorTag))
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(host.name)
                Text("\(host.username)@\(host.hostname):\(host.port)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .tag(host.id)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { onConnect(host) }
        .contextMenu {
            Button("Connect", systemImage: "bolt.fill") { onConnect(host) }
            Divider()
            Button("Edit…", systemImage: "pencil") { sheetTarget = .editHost(host) }
            Button("Duplicate", systemImage: "plus.square.on.square") { duplicate(host) }
            Divider()
            Button("Delete", systemImage: "trash", role: .destructive) {
                pendingDeletion = .host(host)
            }
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
            FolderNameSheet(title: "New Folder") { name in
                store.addFolder(HostFolder(name: name, parentFolderID: parentID))
            }
        case .renameFolder(let folder):
            FolderNameSheet(title: "Rename Folder", initialName: folder.name) { name in
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
            try? KeychainService.save(account: saved.keychainAccount, secret: secret ?? "")
        }
        if case .privateKey = host.authMethod, let secret = try? KeychainService.read(account: host.passphraseAccount) {
            try? KeychainService.save(account: saved.passphraseAccount, secret: secret ?? "")
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

/// Tiny reusable sheet for naming/renaming a folder.
private struct FolderNameSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    var initialName: String = ""
    let onSave: (String) -> Void

    @State private var name: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            TextField("Folder name", text: $name)
                .textFieldStyle(.roundedBorder)
                .onSubmit(save)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding()
        .frame(width: 320)
        .onAppear { name = initialName }
    }

    private func save() {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        onSave(trimmed)
        dismiss()
    }
}
