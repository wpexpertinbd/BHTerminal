import Foundation
import Citadel

/// UI-facing view of one remote directory entry. Wraps Citadel's
/// SFTPPathComponent — SFTP attributes don't expose a convenience
/// isDirectory flag, so it's derived from the POSIX mode bits (falling
/// back to the ls -l style `longname` when a server omits mode bits).
struct SFTPEntry: Identifiable, Hashable {
    var id: String { name }
    let name: String
    let isDirectory: Bool
    let isSymlink: Bool
    let size: UInt64?
    let modificationDate: Date?
    let permissions: UInt32?

    init(component: SFTPPathComponent) {
        self.name = component.filename
        let mode = component.attributes.permissions
        self.isDirectory = SFTPEntry.isDirectory(mode: mode, longname: component.longname)
        self.isSymlink = SFTPEntry.isSymlink(mode: mode, longname: component.longname)
        self.size = component.attributes.size
        self.modificationDate = component.attributes.accessModificationTime?.modificationTime
        self.permissions = mode
    }

    private static func isDirectory(mode: UInt32?, longname: String) -> Bool {
        if let mode { return (mode & 0o170000) == 0o040000 }
        return longname.first == "d"
    }

    private static func isSymlink(mode: UInt32?, longname: String) -> Bool {
        if let mode { return (mode & 0o170000) == 0o120000 }
        return longname.first == "l"
    }
}
