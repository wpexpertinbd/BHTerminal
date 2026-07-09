import SwiftUI

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
                        onPaneExit: { paneID in closePane(paneID, inTabAt: index) }
                    )
                    .id(tabs[index].id)
                }
            }
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
