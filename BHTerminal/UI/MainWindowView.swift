import SwiftUI
import SwiftTerm
import Citadel

/// Sessions | SFTP browser | Terminal — the MobaXterm-style three-column shell.
/// The session sidebar is real (persisted hosts/folders); SFTP and terminal
/// panes are still placeholders until those milestones land.
struct MainWindowView: View {
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var store = SessionStore()

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SessionSidebarView(store: store) { host in
                // Wired to real terminal launch once the terminal core lands.
                print("Connect requested: \(host.name)")
            }
            .navigationTitle("Sessions")
            .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 340)
        } content: {
            SFTPBrowserPlaceholder()
                .navigationSplitViewColumnWidth(min: 260, ideal: 360, max: 600)
        } detail: {
            TerminalPlaceholder()
        }
        .navigationSplitViewStyle(.balanced)
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

private struct TerminalPlaceholder: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 40))
                .foregroundStyle(.secondary)
            Text("Terminal")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.black.opacity(0.85))
        .navigationTitle("Terminal")
    }
}

#Preview {
    MainWindowView()
}
