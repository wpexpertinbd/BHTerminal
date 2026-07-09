import Foundation

/// A folder in the session tree. `parentFolderID == nil` means top-level.
struct HostFolder: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    var name: String
    var parentFolderID: UUID?
    var sortOrder: Int = 0
}
