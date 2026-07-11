import Foundation

/// UI-facing view of one remote directory entry, built from the SFTP
/// protocol client's directory listing. `isDirectory`/`isSymlink` come from
/// the POSIX mode bits (already derived by the protocol client, which falls
/// back to the `ls -l`-style longname when a server omits mode bits).
struct SFTPEntry: Identifiable, Hashable {
    var id: String { name }
    let name: String
    let isDirectory: Bool
    let isSymlink: Bool
    let size: UInt64?
    let modificationDate: Date?
    let permissions: UInt32?

    init(entry: SFTPProtocolClient.Entry) {
        self.name = entry.filename
        self.isDirectory = entry.attributes.isDirectory
        self.isSymlink = entry.attributes.isSymlink
        self.size = entry.attributes.size
        self.modificationDate = entry.attributes.modificationTime
        self.permissions = entry.attributes.permissions
    }
}
