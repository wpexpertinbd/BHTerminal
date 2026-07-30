import AppKit

/// Export/restore flow for BHTerminal's own backup file.
///
/// Everything here is deliberately AppKit (NSAlert + NSSave/OpenPanel) driven
/// from a menu command, NOT a SwiftUI sheet: running a file panel from inside a
/// sheet is what left the sidebar unable to re-render (see HostEditorView's
/// note on .fileImporter). Menu → panel → alert never nests the two.
@MainActor
enum BackupUI {

    // MARK: - Export

    static func export(from store: SessionStore) {
        guard !store.hosts.isEmpty || !store.folders.isEmpty || !store.snippets.isEmpty else {
            alert(style: .informational, title: "Nothing to back up",
                  message: "Add a host first — there are no sessions to export yet.")
            return
        }

        guard let choice = askAboutSecrets(hostCount: store.hosts.count) else { return }

        let panel = NSSavePanel()
        panel.title = "Export BHTerminal Backup"
        panel.nameFieldStringValue = defaultFilename()
        panel.allowsOtherFileTypes = true
        panel.message = choice.passphrase == nil
            ? "Saves your hosts, folders and snippets. Passwords are NOT included."
            : "Saves your hosts, folders and snippets plus saved passwords, encrypted with the passphrase you entered."
        guard panel.runModal() == .OK, let url = panel.url else { return }

        let payload = BackupArchive.snapshot(store: store)
        let counts = (hosts: store.hosts.count, folders: store.folders.count, snippets: store.snippets.count)
        let includeSecrets = choice.passphrase != nil
        let passphrase = choice.passphrase

        // Off the main thread: reading the secrets can raise a Keychain
        // authorization prompt, and waiting for that on the main thread froze
        // the whole app ("Application Not Responding") part-way through.
        Task {
            do {
                let result = try await Task.detached(priority: .userInitiated) {
                    try BackupArchive.makeArchive(payload: payload,
                                                  includeSecrets: includeSecrets,
                                                  passphrase: passphrase)
                }.value

                // 0600: even encrypted, this file is the crown jewels — don't
                // leave it group/world readable.
                try result.data.write(to: url, options: .atomic)
                try? FileManager.default.setAttributes([.posixPermissions: 0o600],
                                                       ofItemAtPath: url.path)

                let secretsNote: String
                if !includeSecrets {
                    secretsNote = "Passwords and key passphrases were not included — you'll re-enter those after restoring."
                } else if result.secretsIncluded == 0 {
                    secretsNote = "⚠️ No saved passwords made it in — the Keychain requests were declined. The hosts themselves are safely backed up; export again and choose “Always Allow” to include passwords."
                } else {
                    secretsNote = "\(result.secretsIncluded) saved password\(result.secretsIncluded == 1 ? "" : "s") included, encrypted. Keep the passphrase safe: without it this backup can't be restored."
                }

                alert(style: .informational, title: "Backup saved",
                      message: """
                      \(counts.hosts) host\(counts.hosts == 1 ? "" : "s"), \
                      \(counts.folders) folder\(counts.folders == 1 ? "" : "s") and \
                      \(counts.snippets) snippet\(counts.snippets == 1 ? "" : "s") \
                      written to “\(url.lastPathComponent)”.

                      \(secretsNote)
                      """)
            } catch {
                alert(style: .warning, title: "Export failed",
                      message: (error as? LocalizedError)?.errorDescription ?? error.localizedDescription)
            }
        }
    }

    private struct SecretChoice { let passphrase: String? }

    /// Including secrets is opt-in AND forces a passphrase — a plaintext dump of
    /// SSH passwords would defeat storing them in the Keychain at all.
    private static func askAboutSecrets(hostCount: Int) -> SecretChoice? {
        let prompt = NSAlert()
        prompt.alertStyle = .informational
        prompt.messageText = "Include saved passwords?"
        prompt.informativeText = """
        Your \(hostCount) host\(hostCount == 1 ? "" : "s"), folders and snippets are always included.

        Saved passwords and key passphrases live in your Keychain. They can be included so a restore is complete, but only in an encrypted backup — enter a passphrase to protect it.

        macOS will ask permission to read them: choose “Always Allow”. (BHTerminal isn't notarized by Apple, so macOS re-asks after each update.)
        """
        let field = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.placeholderString = "Passphrase for the backup"
        prompt.accessoryView = field
        prompt.addButton(withTitle: "Include Passwords")      // .alertFirstButtonReturn
        prompt.addButton(withTitle: "Without Passwords")      // .alertSecondButtonReturn
        prompt.addButton(withTitle: "Cancel")                 // .alertThirdButtonReturn
        prompt.window.initialFirstResponder = field

        switch prompt.runModal() {
        case .alertFirstButtonReturn:
            let passphrase = field.stringValue
            guard passphrase.count >= 8 else {
                alert(style: .warning, title: "Passphrase too short",
                      message: "Use at least 8 characters — this is the only thing protecting your saved passwords.")
                return nil
            }
            return SecretChoice(passphrase: passphrase)
        case .alertSecondButtonReturn:
            return SecretChoice(passphrase: nil)
        default:
            return nil
        }
    }

    private static func defaultFilename() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        return "BHTerminal-Backup-\(formatter.string(from: Date())).json"
    }

    // MARK: - Restore

    static func restore(into store: SessionStore) {
        let panel = NSOpenPanel()
        panel.title = "Restore BHTerminal Backup"
        panel.message = "Choose a BHTerminal backup (.json). Hosts already present are updated; nothing is deleted."
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowsOtherFileTypes = true
        guard panel.runModal() == .OK, let url = panel.url else { return }

        if let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size > 25_000_000 {
            alert(style: .warning, title: "File too large",
                  message: "“\(url.lastPathComponent)” is over 25 MB — far bigger than any backup. Cancelled.")
            return
        }
        guard let data = try? Data(contentsOf: url) else {
            alert(style: .warning, title: "Couldn't read file",
                  message: "“\(url.lastPathComponent)” couldn't be read.")
            return
        }

        do {
            var passphrase: String?
            if try BackupArchive.isEncrypted(data) {
                guard let entered = askPassphrase(filename: url.lastPathComponent) else { return }
                passphrase = entered
            }
            let payload = try BackupArchive.decode(data, passphrase: passphrase)
            let summary = BackupArchive.restore(payload, into: store)

            var lines = ["Hosts: \(summary.hostsAdded) added, \(summary.hostsUpdated) updated",
                         "Folders: \(summary.foldersAdded) added, \(summary.foldersUpdated) updated",
                         "Snippets: \(summary.snippetsAdded) added, \(summary.snippetsUpdated) updated"]
            if summary.secretsRestored > 0 {
                lines.append("Passwords restored: \(summary.secretsRestored)")
            } else if payload.secrets == nil {
                lines.append("This backup had no passwords in it — re-enter them on any password/key hosts.")
            }
            alert(style: .informational, title: "Backup restored", message: lines.joined(separator: "\n"))
        } catch {
            alert(style: .warning, title: "Restore failed",
                  message: (error as? LocalizedError)?.errorDescription ?? error.localizedDescription)
        }
    }

    private static func askPassphrase(filename: String) -> String? {
        let prompt = NSAlert()
        prompt.alertStyle = .informational
        prompt.messageText = "Passphrase required"
        prompt.informativeText = "“\(filename)” is encrypted. Enter the passphrase it was saved with."
        let field = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        prompt.accessoryView = field
        prompt.addButton(withTitle: "Restore")
        prompt.addButton(withTitle: "Cancel")
        prompt.window.initialFirstResponder = field
        guard prompt.runModal() == .alertFirstButtonReturn, !field.stringValue.isEmpty else { return nil }
        return field.stringValue
    }

    // MARK: - Shared

    private static func alert(style: NSAlert.Style, title: String, message: String) {
        let alert = NSAlert()
        alert.alertStyle = style
        alert.messageText = title
        alert.informativeText = message
        alert.runModal()
    }
}
