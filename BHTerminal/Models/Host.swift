import Foundation

/// A saved connection — SSH (with SFTP/terminal/tunnels) or VNC. Never
/// carries a secret directly — passwords and key passphrases live in the
/// Keychain, referenced by `keychainAccount` / `passphraseAccount` so the
/// JSON session file stays safe to back up or sync without leaking
/// credentials.
struct Host: Identifiable, Codable, Hashable {
    enum AuthMethod: Codable, Hashable {
        case password
        case privateKey(path: String)
        case agent
    }

    enum ConnectionType: String, Codable, Hashable {
        case ssh
        case vnc
    }

    var id: UUID = UUID()
    var name: String
    var hostname: String
    var port: Int = 22
    var username: String
    var authMethod: AuthMethod = .agent
    var connectionType: ConnectionType = .ssh
    var jumpHostID: UUID?
    var folderID: UUID?
    var colorTag: String?
    var tunnels: [TunnelRule] = []
    var sortOrder: Int = 0
    var notes: String = ""
    /// VNC only. Off by default — bidirectional clipboard sharing lets a
    /// malicious/MITM'd VNC server read (harvest) or write (poison) the
    /// local clipboard, so it's opt-in per host.
    var vncSharedClipboard: Bool = false

    var keychainAccount: String { Host.keychainAccount(for: id) }
    var passphraseAccount: String { Host.passphraseAccount(for: id) }

    static func keychainAccount(for id: UUID) -> String { "host.\(id.uuidString)" }
    static func passphraseAccount(for id: UUID) -> String { "host.\(id.uuidString).passphrase" }

    init(id: UUID = UUID(), name: String, hostname: String, port: Int = 22, username: String,
         authMethod: AuthMethod = .agent, connectionType: ConnectionType = .ssh,
         jumpHostID: UUID? = nil, folderID: UUID? = nil, colorTag: String? = nil,
         tunnels: [TunnelRule] = [], sortOrder: Int = 0, notes: String = "",
         vncSharedClipboard: Bool = false) {
        self.id = id
        self.name = name
        self.hostname = hostname
        self.port = port
        self.username = username
        self.authMethod = authMethod
        self.connectionType = connectionType
        self.jumpHostID = jumpHostID
        self.folderID = folderID
        self.colorTag = colorTag
        self.tunnels = tunnels
        self.sortOrder = sortOrder
        self.notes = notes
        self.vncSharedClipboard = vncSharedClipboard
    }

    /// Custom decode: `connectionType` is a field added after hosts were
    /// already being saved (including a live-tested one), so it has to be
    /// tolerant of older sessions.json files that don't have the key yet.
    /// SessionStore.load() decodes the whole document with `try?` — one
    /// missing key would otherwise silently drop every saved host, not
    /// just fail to read this one field.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        hostname = try container.decode(String.self, forKey: .hostname)
        port = try container.decode(Int.self, forKey: .port)
        username = try container.decode(String.self, forKey: .username)
        authMethod = try container.decode(AuthMethod.self, forKey: .authMethod)
        connectionType = try container.decodeIfPresent(ConnectionType.self, forKey: .connectionType) ?? .ssh
        jumpHostID = try container.decodeIfPresent(UUID.self, forKey: .jumpHostID)
        folderID = try container.decodeIfPresent(UUID.self, forKey: .folderID)
        colorTag = try container.decodeIfPresent(String.self, forKey: .colorTag)
        tunnels = try container.decodeIfPresent([TunnelRule].self, forKey: .tunnels) ?? []
        sortOrder = try container.decodeIfPresent(Int.self, forKey: .sortOrder) ?? 0
        notes = try container.decodeIfPresent(String.self, forKey: .notes) ?? ""
        vncSharedClipboard = try container.decodeIfPresent(Bool.self, forKey: .vncSharedClipboard) ?? false
    }
}
