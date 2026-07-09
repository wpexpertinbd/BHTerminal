import Foundation

/// A read-only tree view over SessionStore's flat `folders`/`hosts` arrays,
/// built on demand — folders and hosts stay flat and simple to persist, this
/// is purely a presentation-layer convenience for the sidebar.
enum SessionTreeItem: Identifiable, Hashable {
    case folder(HostFolder, children: [SessionTreeItem])
    case host(Host)

    var id: UUID {
        switch self {
        case .folder(let folder, _): return folder.id
        case .host(let host): return host.id
        }
    }

    static func buildTree(folders: [HostFolder], hosts: [Host], parentID: UUID? = nil) -> [SessionTreeItem] {
        let childFolders = folders
            .filter { $0.parentFolderID == parentID }
            .sorted { $0.sortOrder < $1.sortOrder }
            .map { folder in
                SessionTreeItem.folder(folder, children: buildTree(folders: folders, hosts: hosts, parentID: folder.id))
            }
        let childHosts = hosts
            .filter { $0.folderID == parentID }
            .sorted { $0.sortOrder < $1.sortOrder }
            .map { SessionTreeItem.host($0) }
        return childFolders + childHosts
    }
}
