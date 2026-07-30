import Foundation
import Security

enum KeychainError: Error, LocalizedError {
    case unexpectedStatus(OSStatus)
    case unexpectedData

    var errorDescription: String? {
        switch self {
        case .unexpectedStatus(let status):
            return (SecCopyErrorMessageString(status, nil) as String?) ?? "Keychain error \(status)"
        case .unexpectedData:
            return "Keychain returned data in an unexpected format."
        }
    }
}

/// Storage for host passwords and private-key passphrases. SessionStore's JSON
/// only ever holds a reference (the host's UUID) — the secret itself never
/// touches disk in plaintext.
///
/// Everything lives in ONE Keychain entry holding a JSON map of
/// account → secret, rather than one entry per host.
///
/// Why: macOS authorises Keychain reads **per item**, and this app is
/// self-signed, so its access grant doesn't survive an update (the grant is
/// tied to the exact binary). With an entry per secret, 16 hosts meant up to 32
/// separate authorization prompts — backing up turned into an endless stream of
/// password dialogs. One entry means one prompt. It cannot be solved by reading
/// in bulk instead: the file-based keychain rejects `kSecMatchLimitAll` combined
/// with `kSecReturnData` outright (errSecParam, verified), which is exactly how
/// an earlier attempt at this silently exported zero passwords.
///
/// Secrets written by older versions are still read from their individual
/// entries and folded into the vault on first use. Those old entries are left
/// in place rather than deleted, so nothing is destroyed if a build is rolled
/// back.
enum KeychainService {
    private static let service = "com.biswashost.BHTerminal"
    /// Deliberately not a valid host account name ("host.<uuid>").
    private static let vaultAccount = "__bhterminal_secrets__"

    /// Loaded vault, so repeated reads in one launch don't re-authorize.
    /// Guarded because the backup exporter reads off the main thread.
    private static var cache: [String: String]?
    private static let lock = NSLock()

    // MARK: - Public API

    static func save(account: String, secret: String) throws {
        try lock.withLock {
            var vault = try loadVaultLocked()
            vault[account] = secret
            try writeVaultLocked(vault)
        }
    }

    static func read(account: String) throws -> String? {
        try lock.withLock {
            let vault = try loadVaultLocked()
            if let secret = vault[account] { return secret }

            // Written by a version that stored one entry per secret.
            guard let legacy = try readLegacyItemLocked(account: account) else { return nil }
            var updated = vault
            updated[account] = legacy
            try? writeVaultLocked(updated)   // fold it in; failure just means we retry next time
            return legacy
        }
    }

    /// Every secret this app holds — used by the backup exporter.
    ///
    /// The vault costs one authorization; anything still only in a legacy entry
    /// costs one more each, once, as it's migrated. Accounts the user declines
    /// are simply absent, so callers can report what they actually got.
    static func readAll() throws -> [String: String] {
        try lock.withLock {
            var vault = try loadVaultLocked()

            // Listing accounts needs no authorization — only reading their data
            // does — so this never prompts on its own.
            let legacyAccounts = legacyAccountNamesLocked().filter { vault[$0] == nil }
            guard !legacyAccounts.isEmpty else { return vault }

            var migratedAny = false
            for account in legacyAccounts {
                if let secret = try? readLegacyItemLocked(account: account), !secret.isEmpty {
                    vault[account] = secret
                    migratedAny = true
                }
            }
            if migratedAny { try? writeVaultLocked(vault) }
            return vault
        }
    }

    static func delete(account: String) throws {
        try lock.withLock {
            var vault = try loadVaultLocked()
            if vault.removeValue(forKey: account) != nil {
                try writeVaultLocked(vault)
            }
            // Clear any legacy entry too, so a deleted host leaves nothing behind.
            let query: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account
            ]
            let status = SecItemDelete(query as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else {
                throw KeychainError.unexpectedStatus(status)
            }
        }
    }

    // MARK: - Vault

    private static func loadVaultLocked() throws -> [String: String] {
        if let cache { return cache }
        let raw = try readLegacyItemLocked(account: vaultAccount)
        let vault: [String: String]
        if let raw, let data = raw.data(using: .utf8),
           let decoded = try? JSONDecoder().decode([String: String].self, from: data) {
            vault = decoded
        } else {
            vault = [:]
        }
        cache = vault
        return vault
    }

    private static func writeVaultLocked(_ vault: [String: String]) throws {
        let data = try JSONEncoder().encode(vault)
        try writeItemLocked(account: vaultAccount, data: data)
        cache = vault
    }

    // MARK: - Raw item access

    private static func writeItemLocked(account: String, data: Data) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        let updateStatus = SecItemUpdate(query as CFDictionary,
                                         [kSecValueData as String: data] as CFDictionary)
        if updateStatus == errSecItemNotFound {
            var addQuery = query
            addQuery[kSecValueData as String] = data
            // ThisDeviceOnly: never synced to iCloud Keychain and not restorable
            // to another device from an encrypted backup — SSH/VNC credentials
            // should stay on the machine they were entered on.
            addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw KeychainError.unexpectedStatus(addStatus) }
        } else if updateStatus != errSecSuccess {
            throw KeychainError.unexpectedStatus(updateStatus)
        }
    }

    /// Reads one item's data. This is the call that can raise the authorization
    /// prompt — hence the whole point of keeping it to a single item.
    private static func readLegacyItemLocked(account: String) throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let data = item as? Data, let secret = String(data: data, encoding: .utf8) else {
                throw KeychainError.unexpectedData
            }
            return secret
        case errSecItemNotFound:
            return nil
        default:
            throw KeychainError.unexpectedStatus(status)
        }
    }

    /// Account names only — attributes never require authorization, so listing
    /// what exists is free. (Asking for their DATA in the same query is what
    /// the file-based keychain rejects with errSecParam.)
    private static func legacyAccountNamesLocked() -> [String] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let entries = item as? [[String: Any]] else { return [] }
        return entries.compactMap { $0[kSecAttrAccount as String] as? String }
            .filter { $0 != vaultAccount }
    }
}
