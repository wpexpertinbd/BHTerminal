import Foundation

/// A host parsed out of a MobaXterm / SSH-config / Termius export, before it's
/// committed into the store. Kept separate from `Host` so parsing stays pure
/// and unit-testable without touching persistence or the Keychain.
struct ParsedSession: Equatable {
    var name: String
    var hostname: String
    var port: Int
    var username: String
    var connectionType: Host.ConnectionType
    /// Nested folder names (outermost first); empty = directly under the
    /// import's root folder.
    var folderPath: [String]
    /// Only set by the MobaXterm stored-passwords importer; goes straight into
    /// the Keychain, never the JSON store. nil = use agent/key auth.
    var password: String? = nil
}

struct ImportResult {
    var source: String = ""
    var importedHosts = 0
    var importedFolders = 0
    var skipped = 0
    var warnings: [String] = []
}

enum ImportSource: String {
    case mobaXterm = "MobaXterm"
    case mobaXtermPasswords = "MobaXterm Passwords"
    case sshConfig = "SSH Config"
}

enum SessionImportError: LocalizedError {
    case encryptedMobaConf
    case unrecognizedFormat

    var errorDescription: String? {
        switch self {
        case .encryptedMobaConf:
            return """
            This MobaXterm configuration is encrypted with your master password, so its contents can't be read directly.

            Export your sessions unencrypted instead:
            • In MobaXterm, right-click “User sessions” → “Export sessions (.mxtsessions)”, or
            • Settings → remove the master password, then re-export the configuration.

            Then import that file. (Passwords aren't included in the plain export — you'll re-enter them once, and BHTerminal stores them in your macOS Keychain.)
            """
        case .unrecognizedFormat:
            return "Couldn't recognize this file. Supported: MobaXterm sessions (.mxtsessions / plain .mobaconf / MobaXterm.ini) and an OpenSSH config file (~/.ssh/config)."
        }
    }
}

enum SessionImport {
    /// Detects the format and returns the parsed sessions. Throws for an
    /// encrypted MobaXterm config (with guidance) or an unknown format.
    static func parse(contents: String, filename: String) throws -> (source: ImportSource, sessions: [ParsedSession], warnings: [String]) {
        if contents.contains("[Bookmarks") {
            let (sessions, warnings) = MobaXtermImporter.parse(contents)
            return (.mobaXterm, sessions, warnings)
        }

        if looksLikeMobaPasswords(contents) {
            let (sessions, warnings) = MobaXtermPasswordsImporter.parse(contents)
            return (.mobaXtermPasswords, sessions, warnings)
        }

        let ext = (filename as NSString).pathExtension.lowercased()
        if ["mobaconf", "mxtsessions", "ini"].contains(ext) {
            // A MobaXterm file that has no [Bookmarks] section is the
            // master-password-encrypted blob.
            throw SessionImportError.encryptedMobaConf
        }

        if looksLikeSSHConfig(contents) {
            let (sessions, warnings) = SSHConfigImporter.parse(contents)
            return (.sshConfig, sessions, warnings)
        }

        throw SessionImportError.unrecognizedFormat
    }

    private static func looksLikeSSHConfig(_ contents: String) -> Bool {
        contents.components(separatedBy: .newlines).contains { line in
            line.trimmingCharacters(in: .whitespaces).lowercased().hasPrefix("host ")
        }
    }

    /// MobaXterm's "stored passwords" dump: lines of `[proto:]user@host = password`.
    private static func looksLikeMobaPasswords(_ contents: String) -> Bool {
        var matches = 0
        for line in contents.components(separatedBy: .newlines) {
            let t = line.trimmingCharacters(in: .whitespaces)
            guard let eq = t.range(of: " = ") else { continue }
            if t[..<eq.lowerBound].contains("@") {
                matches += 1
                if matches >= 3 { return true }
            }
        }
        return false
    }
}

/// Commits parsed sessions into the store: creates one root folder for the
/// import, recreates the nested folder structure under it, and adds a host
/// per session. Hostnames/usernames are re-validated through SSHSafety here —
/// an import file is exactly the "untrusted host config" vector the security
/// audit flagged, so an option-injection hostname is skipped, not saved.
enum SessionImportCommitter {
    static func commit(source: ImportSource, sessions: [ParsedSession], warnings: [String], into store: SessionStore) -> ImportResult {
        var result = ImportResult(source: source.rawValue, warnings: warnings)

        guard !sessions.isEmpty else {
            result.warnings.append("No sessions were found to import.")
            return result
        }

        // Import can be 700+ hosts — batch so the store's JSON is written once,
        // not once per host.
        store.performBatch {
            commitInBatch(source: source, sessions: sessions, into: store, result: &result)
        }
        return result
    }

    private static func commitInBatch(source: ImportSource, sessions: [ParsedSession], into store: SessionStore, result: inout ImportResult) {
        let root = store.addFolder(HostFolder(name: "\(source.rawValue) Import", parentFolderID: nil))
        result.importedFolders += 1

        var folderCache: [String: UUID] = ["": root.id]
        func ensureFolder(_ path: [String]) -> UUID {
            var parentID = root.id
            var key = ""
            for component in path where !component.isEmpty {
                key += "/" + component
                if let existing = folderCache[key] {
                    parentID = existing
                    continue
                }
                let folder = store.addFolder(HostFolder(name: component, parentFolderID: parentID))
                result.importedFolders += 1
                folderCache[key] = folder.id
                parentID = folder.id
            }
            return parentID
        }

        for session in sessions {
            guard SSHSafety.isValidHostname(session.hostname) else {
                result.skipped += 1
                result.warnings.append("Skipped “\(session.name)” — hostname “\(session.hostname)” is empty or invalid.")
                continue
            }

            var username = session.username
            if session.connectionType == .ssh {
                if username.isEmpty { username = NSUserName() }
                guard SSHSafety.isValidUsername(username) else {
                    result.skipped += 1
                    result.warnings.append("Skipped “\(session.name)” — username “\(username)” is invalid.")
                    continue
                }
            }

            let hasPassword = !(session.password ?? "").isEmpty
            let host = Host(
                name: session.name.isEmpty ? session.hostname : session.name,
                hostname: session.hostname,
                port: session.port,
                username: username,
                // .password only when the import actually carried one (the
                // MobaXterm stored-passwords dump); otherwise agent/key.
                authMethod: hasPassword ? .password : .agent,
                connectionType: session.connectionType,
                folderID: ensureFolder(session.folderPath)
            )
            let saved = store.addHost(host)
            result.importedHosts += 1

            // The secret goes to the Keychain keyed by the new host's id —
            // never into sessions.json.
            if let password = session.password, !password.isEmpty {
                try? KeychainService.save(account: saved.keychainAccount, secret: password)
            }
        }
    }
}
