import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// One pane inside a tab. A tab starts with exactly one; "Split Right"/
/// "Split Down" appends siblings along a single shared axis — a flat grid
/// rather than arbitrarily nested mixed-axis splits, which covers the
/// common MobaXterm case (successive splits) without the complexity of a
/// general recursive-tree reconciler fighting SwiftUI's view diffing.
struct TerminalTabPane: Identifiable, Hashable {
    let id = UUID()
    var host: Host
}

struct TerminalTab: Identifiable {
    let id = UUID()
    var title: String
    var panes: [TerminalTabPane]
    var axis: Axis = .horizontal
}

/// Tab bar + the active tab's split-pane terminal grid.
struct TerminalContainerView: View {
    let store: SessionStore
    @Binding var tabs: [TerminalTab]
    @Binding var selectedTabID: UUID?
    var onCwdChange: (Host, String) -> Void = { _, _ in }

    @State private var broadcastEnabled = false
    @State private var commandDispatch: PaneCommandDispatch?
    @State private var isLoggingActive = false
    @State private var isManagingSnippets = false

    private var selectedTabIndex: Int? {
        tabs.firstIndex { $0.id == selectedTabID }
    }

    var body: some View {
        VStack(spacing: 0) {
            if tabs.isEmpty {
                emptyState
            } else {
                tabBar
                Divider()
                if let index = selectedTabIndex {
                    SplitPaneView(
                        panes: tabs[index].panes,
                        axis: tabs[index].axis,
                        store: store,
                        broadcastEnabled: broadcastEnabled,
                        commandDispatch: commandDispatch,
                        onPaneExit: { paneID in closePane(paneID, inTabAt: index) },
                        onCwdChange: { paneID, path in
                            if let host = tabs[index].panes.first(where: { $0.id == paneID })?.host {
                                onCwdChange(host, path)
                            }
                        }
                    )
                    .id(tabs[index].id)
                }
            }
        }
        .sheet(isPresented: $isManagingSnippets) {
            SnippetManagerView(store: store)
        }
        .onChange(of: selectedTabID) {
            // Logging is a per-pane, per-session concern (see PTYSession) —
            // the toolbar indicator just can't know a newly-selected tab's
            // real state without threading it back out of SplitPaneView's
            // Coordinator, so it resets rather than showing a stale value.
            // The original pane keeps logging correctly regardless.
            isLoggingActive = false
        }
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("Double-click a host in the sidebar to connect")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.black.opacity(0.85))
    }

    private var tabBar: some View {
        HStack(spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) {
                    ForEach(tabs) { tab in
                        tabChip(tab)
                    }
                }
            }
            Spacer(minLength: 8)
            if let index = selectedTabIndex {
                Button {
                    broadcastEnabled.toggle()
                } label: {
                    Image(systemName: broadcastEnabled ? "antenna.radiowaves.left.and.right.circle.fill" : "antenna.radiowaves.left.and.right")
                        .foregroundStyle(broadcastEnabled ? Color.accentColor : .primary)
                }
                .buttonStyle(.borderless)
                .disabled(tabs[index].panes.count < 2)
                .help("Broadcast typed input to every pane in this tab")

                Menu {
                    if store.snippets.isEmpty {
                        Text("No snippets yet")
                    } else {
                        ForEach(store.snippets.sorted { $0.sortOrder < $1.sortOrder }) { snippet in
                            Button(snippet.name) { sendSnippet(snippet.command) }
                        }
                        Divider()
                    }
                    Button("Manage Snippets…") { isManagingSnippets = true }
                } label: {
                    Image(systemName: "text.badge.plus")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("Send a saved snippet to the active pane")

                Button {
                    toggleLogging()
                } label: {
                    Image(systemName: isLoggingActive ? "record.circle.fill" : "record.circle")
                        .foregroundStyle(isLoggingActive ? .red : .primary)
                }
                .buttonStyle(.borderless)
                .help(isLoggingActive ? "Stop logging this session" : "Log this session to a file")

                Menu {
                    Button("Split Right", systemImage: "rectangle.split.2x1") { split(.horizontal, tabIndex: index) }
                    Button("Split Down", systemImage: "rectangle.split.1x2") { split(.vertical, tabIndex: index) }
                } label: {
                    Image(systemName: "square.split.2x1")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .disabled(tabs[index].panes.count >= 4)
                .help("Split this tab")
            }
        }
        .padding(.horizontal, 6)
        .frame(height: 32)
        .background(.bar)
    }

    private func sendSnippet(_ command: String) {
        commandDispatch = PaneCommandDispatch(command: .sendText(command))
    }

    private func toggleLogging() {
        if isLoggingActive {
            commandDispatch = PaneCommandDispatch(command: .stopLogging)
            isLoggingActive = false
            return
        }

        let panel = NSSavePanel()
        panel.nameFieldStringValue = defaultLogFileName()
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        commandDispatch = PaneCommandDispatch(command: .startLogging(url))
        isLoggingActive = true
    }

    private func defaultLogFileName() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd-HHmmss"
        let host = selectedTabIndex.flatMap { tabs[$0].panes.first?.host.name } ?? "session"
        return "\(host)-\(formatter.string(from: Date())).log"
    }

    private func tabChip(_ tab: TerminalTab) -> some View {
        HStack(spacing: 6) {
            Text(tab.title)
                .lineLimit(1)
                .font(.system(size: 12))
            Button {
                closeTab(tab.id)
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(tab.id == selectedTabID ? Color.accentColor.opacity(0.25) : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
        .onTapGesture { selectedTabID = tab.id }
    }

    private func split(_ axis: Axis, tabIndex: Int) {
        guard let host = tabs[tabIndex].panes.first?.host else { return }
        tabs[tabIndex].axis = axis
        tabs[tabIndex].panes.append(TerminalTabPane(host: host))
    }

    private func closePane(_ paneID: UUID, inTabAt index: Int) {
        guard tabs.indices.contains(index) else { return }
        tabs[index].panes.removeAll { $0.id == paneID }
        if tabs[index].panes.isEmpty {
            closeTab(tabs[index].id)
        }
    }

    private func closeTab(_ id: UUID) {
        tabs.removeAll { $0.id == id }
        if selectedTabID == id {
            selectedTabID = tabs.last?.id
        }
    }
}
