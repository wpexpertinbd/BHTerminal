import Foundation

/// A saved SSH/SFTP connection. Never carries a secret directly — passwords
/// and key passphrases live in the Keychain, referenced by `keychainAccount`
/// / `passphraseAccount` so the JSON session file stays safe to back up or
/// sync without leaking credentials.
struct Host: Identifiable, Codable, Hashable {
    enum AuthMethod: Codable, Hashable {
        case password
        case privateKey(path: String)
        case agent
    }

    var id: UUID = UUID()
    var name: String
    var hostname: String
    var port: Int = 22
    var username: String
    var authMethod: AuthMethod = .agent
    var jumpHostID: UUID?
    var folderID: UUID?
    var colorTag: String?
    var tunnels: [TunnelRule] = []
    var sortOrder: Int = 0
    var notes: String = ""

    var keychainAccount: String { Host.keychainAccount(for: id) }
    var passphraseAccount: String { Host.passphraseAccount(for: id) }

    static func keychainAccount(for id: UUID) -> String { "host.\(id.uuidString)" }
    static func passphraseAccount(for id: UUID) -> String { "host.\(id.uuidString).passphrase" }
}
