import Foundation
import Observation

/// Single source of truth for saved hosts/folders/snippets. Backed by one
/// JSON file (`~/Library/Application Support/BHTerminal/sessions.json`),
/// written atomically on every mutation. No secrets ever live here —
/// see KeychainService for those.
@Observable
final class SessionStore {
    private(set) var folders: [HostFolder] = []
    private(set) var hosts: [Host] = []
    private(set) var snippets: [Snippet] = []

    let fileURL: URL

    init(fileURL: URL? = nil) {
        if let fileURL {
            self.fileURL = fileURL
        } else {
            let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            let dir = appSupport.appendingPathComponent("BHTerminal", isDirectory: true)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            self.fileURL = dir.appendingPathComponent("sessions.json")
        }
        load()
    }

    // MARK: - Persistence

    private struct Document: Codable {
        var folders: [HostFolder] = []
        var hosts: [Host] = []
        var snippets: [Snippet] = []
    }

    func load() {
        guard let data = try? Data(contentsOf: fileURL),
              let document = try? JSONDecoder().decode(Document.self, from: data) else { return }
        folders = document.folders
        hosts = document.hosts
        snippets = document.snippets
    }

    func save() {
        let document = Document(folders: folders, hosts: hosts, snippets: snippets)
        guard let data = try? SessionStore.encoder.encode(document) else { return }
        let tempURL = fileURL.appendingPathExtension("tmp")
        do {
            try data.write(to: tempURL, options: .atomic)
            _ = try FileManager.default.replaceItemAt(fileURL, withItemAt: tempURL)
        } catch {
            try? data.write(to: fileURL, options: .atomic)
        }
        // Hostnames/usernames aren't secrets but are still worth keeping off
        // other local accounts — restrict to the owner (0600).
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: fileURL.path)
    }

    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()

    // MARK: - Host CRUD

    @discardableResult
    func addHost(_ host: Host) -> Host {
        var host = host
        host.sortOrder = (hosts.filter { $0.folderID == host.folderID }.map(\.sortOrder).max() ?? -1) + 1
        hosts.append(host)
        save()
        return host
    }

    func updateHost(_ host: Host) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        hosts[index] = host
        save()
    }

    func deleteHost(_ id: UUID) {
        hosts.removeAll { $0.id == id }
        try? KeychainService.delete(account: Host.keychainAccount(for: id))
        try? KeychainService.delete(account: Host.passphraseAccount(for: id))
        save()
    }

    // MARK: - Folder CRUD

    @discardableResult
    func addFolder(_ folder: HostFolder) -> HostFolder {
        var folder = folder
        folder.sortOrder = (folders.filter { $0.parentFolderID == folder.parentFolderID }.map(\.sortOrder).max() ?? -1) + 1
        folders.append(folder)
        save()
        return folder
    }

    func updateFolder(_ folder: HostFolder) {
        guard let index = folders.firstIndex(where: { $0.id == folder.id }) else { return }
        folders[index] = folder
        save()
    }

    /// Deletes a folder and cascades into its subfolders and hosts.
    func deleteFolder(_ id: UUID) {
        let subfolderIDs = folders.filter { $0.parentFolderID == id }.map(\.id)
        subfolderIDs.forEach { deleteFolder($0) }

        let hostIDsInFolder = hosts.filter { $0.folderID == id }.map(\.id)
        hostIDsInFolder.forEach { hostID in
            hosts.removeAll { $0.id == hostID }
            try? KeychainService.delete(account: Host.keychainAccount(for: hostID))
            try? KeychainService.delete(account: Host.passphraseAccount(for: hostID))
        }

        folders.removeAll { $0.id == id }
        save()
    }

    // MARK: - Snippet CRUD

    @discardableResult
    func addSnippet(_ snippet: Snippet) -> Snippet {
        var snippet = snippet
        snippet.sortOrder = (snippets.map(\.sortOrder).max() ?? -1) + 1
        snippets.append(snippet)
        save()
        return snippet
    }

    func updateSnippet(_ snippet: Snippet) {
        guard let index = snippets.firstIndex(where: { $0.id == snippet.id }) else { return }
        snippets[index] = snippet
        save()
    }

    func deleteSnippet(_ id: UUID) {
        snippets.removeAll { $0.id == id }
        save()
    }

    // MARK: - Tree

    var tree: [SessionTreeItem] {
        SessionTreeItem.buildTree(folders: folders, hosts: hosts)
    }
}
