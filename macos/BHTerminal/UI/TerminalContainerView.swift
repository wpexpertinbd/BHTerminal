import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// One pane inside a terminal tab. A tab starts with exactly one; "Split
/// Right"/"Split Down" appends siblings along a single shared axis — a
/// flat grid rather than arbitrarily nested mixed-axis splits, which
/// covers the common MobaXterm case (successive splits) without the
/// complexity of a general recursive-tree reconciler fighting SwiftUI's
/// view diffing.
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

/// A VNC tab is a single view, no splits/broadcast/snippets/logging — same
/// simpler treatment MobaXterm itself gives non-terminal session types.
struct VNCTab: Identifiable {
    let id = UUID()
    var host: Host
    var title: String
}

/// One shared tab bar hosts both kinds; terminal-only toolbar controls
/// (broadcast/snippets/logging/split) only render when the selected tab
/// is a terminal tab.
enum WorkspaceTab: Identifiable {
    case terminal(TerminalTab)
    case vnc(VNCTab)

    var id: UUID {
        switch self {
        case .terminal(let tab): return tab.id
        case .vnc(let tab): return tab.id
        }
    }

    var title: String {
        switch self {
        case .terminal(let tab): return tab.title
        case .vnc(let tab): return tab.title
        }
    }
}

/// Tab bar + the active tab's content (terminal split-pane grid, or a
/// single VNC view).
struct TerminalContainerView: View {
    let store: SessionStore
    @Binding var tabs: [WorkspaceTab]
    @Binding var selectedTabID: UUID?
    var onCwdChange: (Host, String) -> Void = { _, _ in }
    var onReady: (Host) -> Void = { _ in }

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
                    content(for: tabs[index], index: index)
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

    @ViewBuilder
    private func content(for tab: WorkspaceTab, index: Int) -> some View {
        switch tab {
        case .terminal(let terminalTab):
            SplitPaneView(
                panes: terminalTab.panes,
                axis: terminalTab.axis,
                store: store,
                broadcastEnabled: broadcastEnabled,
                commandDispatch: commandDispatch,
                onPaneExit: { paneID in closePane(paneID, inTabAt: index) },
                onCwdChange: { paneID, path in
                    if let host = terminalTab.panes.first(where: { $0.id == paneID })?.host {
                        onCwdChange(host, path)
                    }
                },
                onReady: { paneID in
                    if let host = terminalTab.panes.first(where: { $0.id == paneID })?.host {
                        onReady(host)
                    }
                }
            )
        case .vnc(let vncTab):
            VNCTabView(host: vncTab.host)
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
            if let index = selectedTabIndex, case .terminal(let terminalTab) = tabs[index] {
                Button {
                    broadcastEnabled.toggle()
                } label: {
                    Image(systemName: broadcastEnabled ? "antenna.radiowaves.left.and.right.circle.fill" : "antenna.radiowaves.left.and.right")
                        .foregroundStyle(broadcastEnabled ? Color.accentColor : .primary)
                }
                .buttonStyle(.borderless)
                .disabled(terminalTab.panes.count < 2)
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
                    toggleLogging(terminalTab)
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
                .disabled(terminalTab.panes.count >= 4)
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

    private func toggleLogging(_ terminalTab: TerminalTab) {
        if isLoggingActive {
            commandDispatch = PaneCommandDispatch(command: .stopLogging)
            isLoggingActive = false
            return
        }

        let panel = NSSavePanel()
        panel.nameFieldStringValue = defaultLogFileName(for: terminalTab)
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        commandDispatch = PaneCommandDispatch(command: .startLogging(url))
        isLoggingActive = true
    }

    private func defaultLogFileName(for terminalTab: TerminalTab) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd-HHmmss"
        let host = terminalTab.panes.first?.host.name ?? "session"
        return "\(host)-\(formatter.string(from: Date())).log"
    }

    private func tabChip(_ tab: WorkspaceTab) -> some View {
        HStack(spacing: 6) {
            if case .vnc = tab {
                Image(systemName: "display")
                    .font(.system(size: 10))
            }
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
        guard case .terminal(var terminalTab) = tabs[tabIndex], let host = terminalTab.panes.first?.host else { return }
        terminalTab.axis = axis
        terminalTab.panes.append(TerminalTabPane(host: host))
        tabs[tabIndex] = .terminal(terminalTab)
    }

    private func closePane(_ paneID: UUID, inTabAt index: Int) {
        guard tabs.indices.contains(index), case .terminal(var terminalTab) = tabs[index] else { return }
        terminalTab.panes.removeAll { $0.id == paneID }
        if terminalTab.panes.isEmpty {
            closeTab(terminalTab.id)
        } else {
            tabs[index] = .terminal(terminalTab)
        }
    }

    private func closeTab(_ id: UUID) {
        tabs.removeAll { $0.id == id }
        if selectedTabID == id {
            selectedTabID = tabs.last?.id
        }
    }
}
