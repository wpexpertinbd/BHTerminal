import CommonCrypto
import CryptoKit
import Foundation

enum BackupError: LocalizedError {
    case notABackup
    case unsupportedVersion(Int)
    case wrongPassphrase
    case needsPassphrase
    case encryptionFailed

    var errorDescription: String? {
        switch self {
        case .notABackup:
            return "That file isn't a BHTerminal backup."
        case .unsupportedVersion(let version):
            return "This backup was written by a newer version of BHTerminal (format \(version)). Update BHTerminal and try again."
        case .wrongPassphrase:
            return "Wrong passphrase — the backup couldn't be decrypted."
        case .needsPassphrase:
            return "This backup is encrypted and needs its passphrase."
        case .encryptionFailed:
            return "Couldn't encrypt the backup."
        }
    }
}

/// Read/write of BHTerminal's own backup file: every host, folder and snippet,
/// and — only when the user opts in — the saved passwords and key passphrases.
///
/// Secrets normally live in the macOS Keychain and never touch disk. A backup
/// that contained them in the clear would undo that completely, so including
/// them REQUIRES a passphrase and the whole payload is encrypted with
/// AES-GCM under a PBKDF2-derived key. A backup without secrets is plain JSON,
/// which stays inspectable and diffable.
enum BackupArchive {
    static let formatIdentifier = "bhterminal-backup"
    static let currentVersion = 1
    /// OWASP's floor for PBKDF2-HMAC-SHA256.
    static let kdfIterations = 210_000

    struct Payload: Codable {
        var folders: [HostFolder] = []
        var hosts: [Host] = []
        var snippets: [Snippet] = []
        /// Keychain account → secret. Only present in encrypted backups.
        var secrets: [String: String]?
    }

    struct Envelope: Codable {
        var format: String
        var version: Int
        var exportedAt: Date
        var appVersion: String
        var encrypted: Bool
        var kdf: KDF?
        /// Plain backups carry the payload directly…
        var data: Payload?
        /// …encrypted ones carry an AES-GCM sealed box instead.
        var sealed: Data?

        struct KDF: Codable {
            var algorithm: String
            var salt: Data
            var iterations: Int
        }
    }

    // MARK: - Writing

    /// `passphrase` is required when `includeSecrets` is true, and is what the
    /// whole payload is encrypted under.
    static func export(store: SessionStore,
                       includeSecrets: Bool,
                       passphrase: String?) throws -> Data {
        var payload = Payload(folders: store.folders, hosts: store.hosts, snippets: store.snippets)

        if includeSecrets {
            var secrets: [String: String] = [:]
            for host in store.hosts {
                for account in [host.keychainAccount, host.passphraseAccount] {
                    if let secret = try? KeychainService.read(account: account), !secret.isEmpty {
                        secrets[account] = secret
                    }
                }
            }
            payload.secrets = secrets
        }

        return try encode(payload: payload, encrypt: includeSecrets, passphrase: passphrase)
    }

    /// Encoding split out from gathering so it can be tested without a live
    /// store or Keychain.
    static func encode(payload: Payload, encrypt: Bool, passphrase: String?) throws -> Data {
        var payload = payload
        if !encrypt { payload.secrets = nil }
        let includeSecrets = encrypt

        let appVersion = Bundle.main
            .object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"

        var envelope = Envelope(format: formatIdentifier,
                                version: currentVersion,
                                exportedAt: Date(),
                                appVersion: appVersion,
                                encrypted: includeSecrets)

        if includeSecrets {
            guard let passphrase, !passphrase.isEmpty else { throw BackupError.needsPassphrase }
            var salt = Data(count: 16)
            let ok = salt.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress!) }
            guard ok == errSecSuccess else { throw BackupError.encryptionFailed }

            let key = try deriveKey(passphrase: passphrase, salt: salt, iterations: kdfIterations)
            let plaintext = try JSONEncoder.backup.encode(payload)
            guard let box = try? AES.GCM.seal(plaintext, using: key), let combined = box.combined else {
                throw BackupError.encryptionFailed
            }
            envelope.kdf = .init(algorithm: "PBKDF2-HMAC-SHA256", salt: salt, iterations: kdfIterations)
            envelope.sealed = combined
        } else {
            envelope.data = payload
        }

        return try JSONEncoder.backup.encode(envelope)
    }

    // MARK: - Reading

    /// Whether a passphrase is needed before `decode` can succeed.
    static func isEncrypted(_ data: Data) throws -> Bool {
        guard let envelope = try? JSONDecoder.backup.decode(Envelope.self, from: data),
              envelope.format == formatIdentifier else { throw BackupError.notABackup }
        guard envelope.version <= currentVersion else {
            throw BackupError.unsupportedVersion(envelope.version)
        }
        return envelope.encrypted
    }

    static func decode(_ data: Data, passphrase: String?) throws -> Payload {
        guard let envelope = try? JSONDecoder.backup.decode(Envelope.self, from: data),
              envelope.format == formatIdentifier else { throw BackupError.notABackup }
        guard envelope.version <= currentVersion else {
            throw BackupError.unsupportedVersion(envelope.version)
        }

        guard envelope.encrypted else {
            guard let payload = envelope.data else { throw BackupError.notABackup }
            return payload
        }

        guard let passphrase, !passphrase.isEmpty else { throw BackupError.needsPassphrase }
        guard let kdf = envelope.kdf, let sealed = envelope.sealed else { throw BackupError.notABackup }

        let key = try deriveKey(passphrase: passphrase, salt: kdf.salt, iterations: kdf.iterations)
        guard let box = try? AES.GCM.SealedBox(combined: sealed),
              let plaintext = try? AES.GCM.open(box, using: key) else {
            // AES-GCM authenticates, so a failure here is a wrong passphrase or
            // a tampered file — either way the contents can't be trusted.
            throw BackupError.wrongPassphrase
        }
        guard let payload = try? JSONDecoder.backup.decode(Payload.self, from: plaintext) else {
            throw BackupError.wrongPassphrase
        }
        return payload
    }

    // MARK: - Restoring

    struct RestoreSummary {
        var hostsAdded = 0, hostsUpdated = 0
        var foldersAdded = 0, foldersUpdated = 0
        var snippetsAdded = 0, snippetsUpdated = 0
        var secretsRestored = 0
    }

    /// Merges by id: anything missing is added, anything already present is
    /// updated. Deliberately never deletes — restoring a backup shouldn't throw
    /// away hosts added since it was taken.
    @discardableResult
    static func restore(_ payload: Payload, into store: SessionStore) -> RestoreSummary {
        var summary = RestoreSummary()

        // Folders first, so hosts always have their folder to land in.
        for folder in payload.folders {
            if store.folders.contains(where: { $0.id == folder.id }) {
                store.updateFolder(folder)
                summary.foldersUpdated += 1
            } else {
                store.addFolder(folder)
                summary.foldersAdded += 1
            }
        }
        for host in payload.hosts {
            if store.hosts.contains(where: { $0.id == host.id }) {
                store.updateHost(host)
                summary.hostsUpdated += 1
            } else {
                _ = store.addHost(host)
                summary.hostsAdded += 1
            }
        }
        for snippet in payload.snippets {
            if store.snippets.contains(where: { $0.id == snippet.id }) {
                store.updateSnippet(snippet)
                summary.snippetsUpdated += 1
            } else {
                _ = store.addSnippet(snippet)
                summary.snippetsAdded += 1
            }
        }
        for (account, secret) in payload.secrets ?? [:] {
            if (try? KeychainService.save(account: account, secret: secret)) != nil {
                summary.secretsRestored += 1
            }
        }
        return summary
    }

    // MARK: - Key derivation

    private static func deriveKey(passphrase: String, salt: Data, iterations: Int) throws -> SymmetricKey {
        let passwordBytes = Array(passphrase.utf8)
        var derived = [UInt8](repeating: 0, count: 32)
        let status: Int32 = salt.withUnsafeBytes { saltBuffer in
            CCKeyDerivationPBKDF(
                CCPBKDFAlgorithm(kCCPBKDF2),
                passwordBytes.map { Int8(bitPattern: $0) }, passwordBytes.count,
                saltBuffer.bindMemory(to: UInt8.self).baseAddress, salt.count,
                CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256),
                UInt32(iterations),
                &derived, derived.count
            )
        }
        guard status == kCCSuccess else { throw BackupError.encryptionFailed }
        return SymmetricKey(data: Data(derived))
    }
}

private extension JSONEncoder {
    static var backup: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }
}

private extension JSONDecoder {
    static var backup: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}
