import SwiftUI
import UniformTypeIdentifiers

/// The left-pane remote file browser. Connects independently from the
/// terminal (see SFTPConnection) but is driven by the same host selection —
/// MainWindowView connects both together on double-click.
struct SFTPBrowserView: View {
    var connection: SFTPConnection

    @State private var selection: Set<String> = []
    @State private var sheet: Sheet?
    @State private var pendingDelete: SFTPEntry?
    @State private var isDropTargeted = false

    private enum Sheet: Identifiable {
        case newFolder
        case rename(SFTPEntry)

        var id: String {
            switch self {
            case .newFolder: return "newFolder"
            case .rename(let entry): return "rename-\(entry.id)"
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            if connection.connectedHost == nil {
                emptyState
            } else {
                toolbar
                Divider()
                pathBar
                Divider()
                fileList
                Divider()
                followBar
            }
        }
        .overlay(alignment: .top) {
            if let error = connection.errorMessage {
                errorBanner(error)
            }
        }
        .overlay {
            if connection.entries.isEmpty, let status = connection.statusMessage {
                VStack(spacing: 10) {
                    ProgressView()
                    Text(status).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .sheet(item: $sheet) { sheet in
            sheetContent(sheet)
        }
        .confirmationDialog(
            pendingDelete.map { "Delete “\($0.name)”?" } ?? "",
            isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                if let entry = pendingDelete {
                    Task { await connection.delete(entry) }
                }
                pendingDelete = nil
            }
            Button("Cancel", role: .cancel) { pendingDelete = nil }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            Image(systemName: "folder")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("Double-click a host to browse its files")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func errorBanner(_ message: String) -> some View {
        Text(message)
            .font(.caption)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.red.opacity(0.15))
            .overlay(Divider(), alignment: .bottom)
    }

    // MARK: - Toolbar

    private var toolbar: some View {
        HStack(spacing: 10) {
            Button { Task { await connection.navigateUp() } } label: {
                Image(systemName: "chevron.up")
            }
            .disabled(connection.currentPath == "/")

            Button { Task { await connection.refresh() } } label: {
                Image(systemName: "arrow.clockwise")
            }

            Divider().frame(height: 16)

            Button { sheet = .newFolder } label: {
                Image(systemName: "folder.badge.plus")
            }

            Button { chooseFilesToUpload() } label: {
                Image(systemName: "arrow.up.doc")
            }
            .help("Upload…")

            if let entry = singleSelectedEntry {
                Button { sheet = .rename(entry) } label: {
                    Image(systemName: "pencil")
                }
                .help("Rename")

                if !entry.isDirectory {
                    Button { downloadEntry(entry) } label: {
                        Image(systemName: "arrow.down.doc")
                    }
                    .help("Download…")
                }
            }

            if !selection.isEmpty {
                Button(role: .destructive) {
                    if selection.count == 1, let entry = singleSelectedEntry {
                        pendingDelete = entry
                    }
                } label: {
                    Image(systemName: "trash")
                }
                .help("Delete")
            }

            Spacer()

            if connection.isLoading {
                ProgressView().controlSize(.small)
            }

            Button { Task { await connection.disconnect() } } label: {
                Image(systemName: "eject")
            }
            .help("Disconnect SFTP")
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 8)
        .frame(height: 32)
        .background(.bar)
    }

    /// Bottom bar: opt-in cwd-follow.
    private var followBar: some View {
        HStack {
            Toggle(isOn: Binding(
                get: { connection.followTerminal },
                set: { connection.setFollowTerminal($0) }
            )) {
                Text("Follow Terminal").font(.caption)
            }
            .toggleStyle(.checkbox)
            .help("Keep this browser in the same folder the terminal is currently in")
            Spacer()
        }
        .padding(.horizontal, 8)
        .frame(height: 28)
        .background(.bar)
    }

    private var pathBar: some View {
        Text(connection.currentPath)
            .font(.system(.caption, design: .monospaced))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.head)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var singleSelectedEntry: SFTPEntry? {
        guard selection.count == 1, let id = selection.first else { return nil }
        return connection.entries.first { $0.id == id }
    }

    // MARK: - File list

    private var fileList: some View {
        List(selection: $selection) {
            ForEach(connection.entries) { entry in
                row(for: entry)
                    .tag(entry.id)
            }
        }
        .listStyle(.inset)
        .contextMenu(forSelectionType: String.self) { ids in
            contextMenuItems(for: ids)
        } primaryAction: { ids in
            if ids.count == 1, let id = ids.first, let entry = connection.entries.first(where: { $0.id == id }), entry.isDirectory {
                Task { await connection.navigate(to: entry) }
            }
        }
        .onDrop(of: [.fileURL], isTargeted: $isDropTargeted) { providers in
            handleDrop(providers)
        }
        .overlay {
            if isDropTargeted {
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.accentColor, lineWidth: 2)
                    .padding(4)
            }
        }
    }

    @ViewBuilder
    private func contextMenuItems(for ids: Set<String>) -> some View {
        if ids.count == 1, let id = ids.first, let entry = connection.entries.first(where: { $0.id == id }) {
            if entry.isDirectory {
                Button("Open", systemImage: "folder") {
                    Task { await connection.navigate(to: entry) }
                }
            } else {
                Button("Download…", systemImage: "arrow.down.doc") { downloadEntry(entry) }
            }
            Button("Rename…", systemImage: "pencil") { sheet = .rename(entry) }
            Divider()
            Button("Delete", systemImage: "trash", role: .destructive) { pendingDelete = entry }
        }
    }

    private func row(for entry: SFTPEntry) -> some View {
        HStack(spacing: 8) {
            Image(systemName: icon(for: entry))
                .foregroundStyle(entry.isDirectory ? Color.accentColor : .secondary)
                .frame(width: 18)
            Text(entry.name)
                .lineLimit(1)
            Spacer()
            if !entry.isDirectory, let size = entry.size {
                Text(formattedSize(size))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        // Whole-width hit area for selection; single-click selects (List),
        // double-click navigates (the List's primaryAction). No tap gesture
        // here — one would swallow the click and kill the selection highlight.
        .contentShape(Rectangle())
    }

    private func icon(for entry: SFTPEntry) -> String {
        if entry.isDirectory { return "folder.fill" }
        if entry.isSymlink { return "link" }
        return "doc"
    }

    private func formattedSize(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    // MARK: - Sheets

    @ViewBuilder
    private func sheetContent(_ sheet: Sheet) -> some View {
        switch sheet {
        case .newFolder:
            TextPromptSheet(title: "New Folder", placeholder: "Folder name") { name in
                Task { await connection.createDirectory(name: name) }
            }
        case .rename(let entry):
            TextPromptSheet(title: "Rename", initialValue: entry.name, placeholder: "New name") { name in
                Task { await connection.rename(entry, to: name) }
            }
        }
    }

    // MARK: - Transfers

    private func chooseFilesToUpload() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        for url in panel.urls {
            Task { await connection.upload(from: url) }
        }
    }

    private func downloadEntry(_ entry: SFTPEntry) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = entry.name
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await connection.download(entry, to: url) }
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        var didAccept = false
        for provider in providers where provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            didAccept = true
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier) { item, _ in
                guard let data = item as? Data,
                      let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
                Task { @MainActor in
                    await connection.upload(from: url)
                }
            }
        }
        return didAccept
    }
}
