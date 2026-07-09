import Foundation

/// A saved command the user can send to the active terminal with one click.
struct Snippet: Identifiable, Codable, Hashable {
    var id: UUID = UUID()
    var name: String
    var command: String
    var sortOrder: Int = 0
}
