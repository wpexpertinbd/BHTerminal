import XCTest
@testable import BHTerminal

/// Backup/restore is the one feature where a bug loses the user's data, so the
/// round-trip and the encryption guarantees are pinned down here.
final class BackupArchiveTests: XCTestCase {

    private func samplePayload() -> BackupArchive.Payload {
        let folder = HostFolder(name: "BiswasHost")
        var host = Host(name: "s1", hostname: "s1.example.com", port: 2279,
                        username: "root", authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        host.folderID = folder.id
        return BackupArchive.Payload(folders: [folder], hosts: [host],
                                     snippets: [Snippet(name: "df", command: "df -h")],
                                     secrets: ["host.\(host.id.uuidString)": "s3cret-passphrase"])
    }

    private func envelope(_ payload: BackupArchive.Payload,
                          encrypted: Bool, passphrase: String?) throws -> Data {
        // encode() is the real writing path; export() only adds gathering from
        // a live store/Keychain on top of it.
        try BackupArchive.encode(payload: payload, encrypt: encrypted, passphrase: passphrase)
    }

    func testPlainBackupRoundTrips() throws {
        let payload = samplePayload()
        let data = try envelope(payload, encrypted: false, passphrase: nil)

        XCTAssertFalse(try BackupArchive.isEncrypted(data))
        let decoded = try BackupArchive.decode(data, passphrase: nil)
        XCTAssertEqual(decoded.hosts.map(\.id), payload.hosts.map(\.id))
        XCTAssertEqual(decoded.folders.map(\.name), ["BiswasHost"])
        XCTAssertEqual(decoded.snippets.map(\.name), ["df"])
        // A backup without secrets must not carry any.
        XCTAssertNil(decoded.secrets)
    }

    func testEncryptedBackupRoundTripsWithTheRightPassphrase() throws {
        let payload = samplePayload()
        let data = try envelope(payload, encrypted: true, passphrase: "correct horse battery")

        XCTAssertTrue(try BackupArchive.isEncrypted(data))
        let decoded = try BackupArchive.decode(data, passphrase: "correct horse battery")
        XCTAssertEqual(decoded.hosts.first?.hostname, "s1.example.com")
        XCTAssertEqual(decoded.secrets?.values.first, "s3cret-passphrase")
    }

    /// The whole point of encrypting: secrets must not be readable in the file.
    func testSecretsNeverAppearInCleartext() throws {
        let data = try envelope(samplePayload(), encrypted: true, passphrase: "a-good-passphrase")
        let text = String(decoding: data, as: UTF8.self)
        XCTAssertFalse(text.contains("s3cret-passphrase"))
        XCTAssertFalse(text.contains("s1.example.com"))
    }

    func testWrongPassphraseIsRejected() throws {
        let data = try envelope(samplePayload(), encrypted: true, passphrase: "the-real-one")
        XCTAssertThrowsError(try BackupArchive.decode(data, passphrase: "not-it")) { error in
            XCTAssertEqual(error as? BackupError, .wrongPassphrase)
        }
        XCTAssertThrowsError(try BackupArchive.decode(data, passphrase: nil)) { error in
            XCTAssertEqual(error as? BackupError, .needsPassphrase)
        }
    }

    /// AES-GCM authenticates, so a tampered backup must fail rather than
    /// restore attacker-chosen hosts.
    func testTamperedBackupIsRejected() throws {
        var data = try envelope(samplePayload(), encrypted: true, passphrase: "a-good-passphrase")
        // Flip a byte inside the base64 sealed box.
        guard let range = String(decoding: data, as: UTF8.self).range(of: "\"sealed\" : \"") else {
            return XCTFail("no sealed field")
        }
        var text = String(decoding: data, as: UTF8.self)
        let idx = text.index(range.upperBound, offsetBy: 8)
        text.replaceSubrange(idx...idx, with: text[idx] == "A" ? "B" : "A")
        data = Data(text.utf8)
        XCTAssertThrowsError(try BackupArchive.decode(data, passphrase: "a-good-passphrase"))
    }

    func testRejectsFilesThatArentBackups() {
        let notABackup = Data(#"{"hello":"world"}"#.utf8)
        XCTAssertThrowsError(try BackupArchive.isEncrypted(notABackup)) { error in
            XCTAssertEqual(error as? BackupError, .notABackup)
        }
    }
}

extension BackupError: Equatable {
    public static func == (lhs: BackupError, rhs: BackupError) -> Bool {
        String(describing: lhs) == String(describing: rhs)
    }
}
