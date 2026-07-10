import SwiftUI

/// Add/edit/delete saved snippets (CRUD already lives on SessionStore).
/// Sending a snippet to the active terminal pane is handled by
/// TerminalContainerView's Snippets menu — this sheet is purely management.
struct SnippetManagerView: View {
    @Environment(\.dismiss) private var dismiss
    let store: SessionStore

    @State private var sheet: Sheet?

    private enum Sheet: Identifiable {
        case new
        case edit(Snippet)

        var id: String {
            switch self {
            case .new: return "new"
            case .edit(let snippet): return "edit-\(snippet.id)"
            }
        }
    }

    private var sortedSnippets: [Snippet] {
        store.snippets.sorted { $0.sortOrder < $1.sortOrder }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Snippets").font(.headline)
                Spacer()
                Button("Add…") { sheet = .new }
            }
            .padding()

            Divider()

            if store.snippets.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "text.quote")
                        .font(.system(size: 32))
                        .foregroundStyle(.secondary)
                    Text("No snippets yet")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(sortedSnippets) { snippet in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(snippet.name)
                                Text(snippet.command)
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                            Spacer()
                            Button("Edit…") { sheet = .edit(snippet) }
                                .buttonStyle(.borderless)
                            Button("Delete", role: .destructive) { store.deleteSnippet(snippet.id) }
                                .buttonStyle(.borderless)
                        }
                    }
                }
            }

            Divider()
            HStack {
                Spacer()
                Button("Close") { dismiss() }
            }
            .padding()
        }
        .frame(width: 420, height: 360)
        .sheet(item: $sheet) { sheetContent(for: $0) }
    }

    @ViewBuilder
    private func sheetContent(for sheet: Sheet) -> some View {
        switch sheet {
        case .new:
            SnippetEditorSheet { name, command in
                store.addSnippet(Snippet(name: name, command: command))
            }
        case .edit(let snippet):
            SnippetEditorSheet(initialName: snippet.name, initialCommand: snippet.command) { name, command in
                var updated = snippet
                updated.name = name
                updated.command = command
                store.updateSnippet(updated)
            }
        }
    }
}

private struct SnippetEditorSheet: View {
    @Environment(\.dismiss) private var dismiss
    var initialName: String = ""
    var initialCommand: String = ""
    let onSave: (String, String) -> Void

    @State private var name: String = ""
    @State private var command: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(initialName.isEmpty ? "New Snippet" : "Edit Snippet").font(.headline)
            TextField("Name", text: $name)
            TextField("Command", text: $command)
                .font(.system(.body, design: .monospaced))
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || command.isEmpty)
            }
        }
        .padding()
        .frame(width: 360)
        .onAppear { name = initialName; command = initialCommand }
    }

    private func save() {
        onSave(name.trimmingCharacters(in: .whitespaces), command)
        dismiss()
    }
}
