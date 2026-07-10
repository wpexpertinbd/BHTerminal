import AppKit

/// The user-facing import flow: pick a file, parse it, commit into the store,
/// and report what happened — shared by the sidebar's "+" menu and the File
/// menu command.
@MainActor
enum SessionImportUI {
    static func run(into store: SessionStore) {
        let panel = NSOpenPanel()
        panel.title = "Import Sessions"
        panel.message = """
        Choose a file to import:
        • MobaXterm passwords export (.txt) — hosts + passwords. In MobaXterm: Settings → Configuration → General → “MobaXterm passwords management” → Export to file.
        • MobaXterm sessions (.mxtsessions) — names + folders, no passwords.
        • OpenSSH config (~/.ssh/config).
        """
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowsOtherFileTypes = true

        guard panel.runModal() == .OK, let url = panel.url else { return }

        guard let contents = readText(url) else {
            alert(style: .warning, title: "Couldn't read file",
                  message: "“\(url.lastPathComponent)” couldn't be read as text.")
            return
        }

        do {
            let (source, sessions, warnings) = try SessionImport.parse(contents: contents, filename: url.lastPathComponent)
            let result = SessionImportCommitter.commit(source: source, sessions: sessions, warnings: warnings, into: store)
            reportSuccess(result)
        } catch {
            alert(style: .warning, title: "Import failed",
                  message: (error as? LocalizedError)?.errorDescription ?? error.localizedDescription)
        }
    }

    /// MobaXterm.ini can be Windows-1252; try UTF-8 first, then fall back.
    private static func readText(_ url: URL) -> String? {
        if let utf8 = try? String(contentsOf: url, encoding: .utf8) { return utf8 }
        if let cp1252 = try? String(contentsOf: url, encoding: .windowsCP1252) { return cp1252 }
        return try? String(contentsOf: url, encoding: .isoLatin1)
    }

    private static func reportSuccess(_ result: ImportResult) {
        let summary = "Imported \(result.importedHosts) host\(result.importedHosts == 1 ? "" : "s") "
            + "into \(result.importedFolders) folder\(result.importedFolders == 1 ? "" : "s")"
            + (result.skipped > 0 ? ", skipped \(result.skipped)." : ".")

        var info = summary
        if !result.warnings.isEmpty {
            let shown = result.warnings.prefix(12).joined(separator: "\n• ")
            info += "\n\n• " + shown
            if result.warnings.count > 12 {
                info += "\n…and \(result.warnings.count - 12) more."
            }
        }
        alert(style: .informational, title: "\(result.source) import complete", message: info)
    }

    private static func alert(style: NSAlert.Style, title: String, message: String) {
        let alert = NSAlert()
        alert.alertStyle = style
        alert.messageText = title
        alert.informativeText = message
        alert.runModal()
    }
}
