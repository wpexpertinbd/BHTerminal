import SwiftUI
import SwiftTerm
import Citadel

/// Sessions | SFTP browser | Terminal — the MobaXterm-style three-column shell.
/// Column contents are placeholders until the persistence, SFTP, and terminal
/// milestones land; this view exists first to prove the project + both SPM
/// dependencies (SwiftTerm, Citadel) actually resolve and link.
struct MainWindowView: View {
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SessionTreePlaceholder()
                .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
        } content: {
            SFTPBrowserPlaceholder()
                .navigationSplitViewColumnWidth(min: 260, ideal: 360, max: 600)
        } detail: {
            TerminalPlaceholder()
        }
        .navigationSplitViewStyle(.balanced)
    }
}

private struct SessionTreePlaceholder: View {
    var body: some View {
        List {
            Label("Sessions", systemImage: "server.rack")
                .foregroundStyle(.secondary)
        }
        .navigationTitle("Sessions")
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
