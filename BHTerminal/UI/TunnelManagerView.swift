import SwiftUI

/// Per-host tunnel list — add/edit/delete rules (persisted on the Host via
/// SessionStore) and start/stop them (via the app-lifetime TunnelManager,
/// so a tunnel keeps running even after this sheet closes).
struct TunnelManagerView: View {
    @Environment(\.dismiss) private var dismiss
    let store: SessionStore
    let manager: TunnelManager
    let host: Host

    @State private var sheet: Sheet?

    private enum Sheet: Identifiable {
        case new
        case edit(TunnelRule)

        var id: String {
            switch self {
            case .new: return "new"
            case .edit(let rule): return "edit-\(rule.id)"
            }
        }
    }

    /// Re-reads the latest saved copy each render — tunnels can be added
    /// while this sheet is open.
    private var currentHost: Host {
        store.hosts.first { $0.id == host.id } ?? host
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Tunnels — \(currentHost.name)").font(.headline)
                Spacer()
                Button("Add…") { sheet = .new }
            }
            .padding()

            Divider()

            if currentHost.tunnels.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "arrow.left.arrow.right")
                        .font(.system(size: 32))
                        .foregroundStyle(.secondary)
                    Text("No tunnels yet")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(currentHost.tunnels) { rule in
                        row(for: rule)
                    }
                }
            }

            Divider()
            HStack {
                if let error = manager.lastError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                }
                Spacer()
                Button("Close") { dismiss() }
            }
            .padding()
        }
        .frame(width: 480, height: 360)
        .sheet(item: $sheet) { sheetContent(for: $0) }
    }

    private func row(for rule: TunnelRule) -> some View {
        HStack {
            Circle()
                .fill(manager.isRunning(rule.id) ? Color.green : Color.secondary.opacity(0.4))
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
                Text(rule.name.isEmpty ? kindLabel(rule.kind) : rule.name)
                Text(detail(for: rule))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(manager.isRunning(rule.id) ? "Stop" : "Start") {
                toggle(rule)
            }
            .controlSize(.small)
            Menu {
                Button("Edit…", systemImage: "pencil") { sheet = .edit(rule) }
                Button("Delete", systemImage: "trash", role: .destructive) { delete(rule) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    private func kindLabel(_ kind: TunnelRule.Kind) -> String {
        switch kind {
        case .local: return "Local Forward"
        case .remote: return "Remote Forward"
        case .dynamic: return "Dynamic (SOCKS)"
        }
    }

    private func detail(for rule: TunnelRule) -> String {
        switch rule.kind {
        case .local, .remote:
            return "\(rule.listenHost):\(rule.listenPort) \u{2192} \(rule.destHost):\(rule.destPort)"
        case .dynamic:
            return "SOCKS on \(rule.listenHost):\(rule.listenPort)"
        }
    }

    private func toggle(_ rule: TunnelRule) {
        if manager.isRunning(rule.id) {
            manager.stop(rule.id)
        } else {
            manager.start(rule, on: currentHost) { jumpID in store.hosts.first { $0.id == jumpID } }
        }
    }

    private func delete(_ rule: TunnelRule) {
        manager.stop(rule.id)
        var updated = currentHost
        updated.tunnels.removeAll { $0.id == rule.id }
        store.updateHost(updated)
    }

    @ViewBuilder
    private func sheetContent(for sheet: Sheet) -> some View {
        switch sheet {
        case .new:
            TunnelRuleEditorView { rule in
                var updated = currentHost
                updated.tunnels.append(rule)
                store.updateHost(updated)
            }
        case .edit(let rule):
            TunnelRuleEditorView(editingRule: rule) { updatedRule in
                var updated = currentHost
                if let index = updated.tunnels.firstIndex(where: { $0.id == rule.id }) {
                    updated.tunnels[index] = updatedRule
                }
                store.updateHost(updated)
            }
        }
    }
}

private struct TunnelRuleEditorView: View {
    @Environment(\.dismiss) private var dismiss
    var editingRule: TunnelRule?
    let onSave: (TunnelRule) -> Void

    @State private var name: String
    @State private var kind: TunnelRule.Kind
    @State private var listenHost: String
    @State private var listenPort: String
    @State private var destHost: String
    @State private var destPort: String

    init(editingRule: TunnelRule? = nil, onSave: @escaping (TunnelRule) -> Void) {
        self.editingRule = editingRule
        self.onSave = onSave
        _name = State(initialValue: editingRule?.name ?? "")
        _kind = State(initialValue: editingRule?.kind ?? .local)
        _listenHost = State(initialValue: editingRule?.listenHost ?? "127.0.0.1")
        _listenPort = State(initialValue: editingRule.map { String($0.listenPort) } ?? "")
        _destHost = State(initialValue: editingRule?.destHost ?? "")
        _destPort = State(initialValue: editingRule.map { String($0.destPort) } ?? "")
    }

    var body: some View {
        Form {
            TextField("Name (optional)", text: $name)
            Picker("Type", selection: $kind) {
                Text("Local").tag(TunnelRule.Kind.local)
                Text("Remote").tag(TunnelRule.Kind.remote)
                Text("Dynamic (SOCKS)").tag(TunnelRule.Kind.dynamic)
            }
            .pickerStyle(.segmented)

            TextField("Listen host", text: $listenHost)
            TextField("Listen port", text: $listenPort)

            if kind != .dynamic {
                TextField("Destination host", text: $destHost)
                TextField("Destination port", text: $destPort)
            }
        }
        .formStyle(.grouped)
        .frame(width: 380, height: kind == .dynamic ? 220 : 300)
        .safeAreaInset(edge: .bottom) {
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button(editingRule == nil ? "Add" : "Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!isValid)
            }
            .padding()
            .background(.bar)
        }
    }

    private var isValid: Bool {
        guard Int(listenPort) != nil else { return false }
        if kind == .dynamic { return true }
        return !destHost.trimmingCharacters(in: .whitespaces).isEmpty && Int(destPort) != nil
    }

    private func save() {
        guard let lp = Int(listenPort) else { return }
        var rule = editingRule ?? TunnelRule(kind: kind, listenPort: lp)
        rule.name = name
        rule.kind = kind
        rule.listenHost = listenHost.isEmpty ? "127.0.0.1" : listenHost
        rule.listenPort = lp
        rule.destHost = destHost
        rule.destPort = Int(destPort) ?? 0
        onSave(rule)
        dismiss()
    }
}
