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

/// Thin wrapper around Keychain Services for host passwords and private-key
/// passphrases. SessionStore's JSON file only ever holds a reference
/// (the host's UUID) — the actual secret never touches disk in plaintext.
enum KeychainService {
    private static let service = "com.biswashost.BHTerminal"

    static func save(account: String, secret: String) throws {
        let data = Data(secret.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]

        let updateStatus = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)

        if updateStatus == errSecItemNotFound {
            var addQuery = query
            addQuery[kSecValueData as String] = data
            // ThisDeviceOnly: never synced to iCloud Keychain and not
            // restorable to another device from an encrypted backup — SSH/VNC
            // credentials should stay on the machine they were entered on.
            addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw KeychainError.unexpectedStatus(addStatus) }
        } else if updateStatus != errSecSuccess {
            throw KeychainError.unexpectedStatus(updateStatus)
        }
    }

    static func read(account: String) throws -> String? {
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

    /// Every stored secret in ONE query, keyed by account.
    ///
    /// Backing up used to call `read(account:)` once per account — with 16
    /// hosts that's 32 separate Keychain requests, each able to raise its own
    /// authorization prompt, which is why exporting turned into an endless
    /// stream of password dialogs. One query is the fewest requests possible.
    ///
    /// Items the user declines are simply absent from the result; the caller
    /// reports how many it actually got rather than pretending it has them all.
    static func readAll() throws -> [String: String] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnData as String: true,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let entries = item as? [[String: Any]] else { return [:] }
            var secrets: [String: String] = [:]
            for entry in entries {
                guard let account = entry[kSecAttrAccount as String] as? String,
                      let data = entry[kSecValueData as String] as? Data,
                      let secret = String(data: data, encoding: .utf8) else { continue }
                secrets[account] = secret
            }
            return secrets
        case errSecItemNotFound:
            return [:]
        default:
            throw KeychainError.unexpectedStatus(status)
        }
    }

    static func delete(account: String) throws {
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
