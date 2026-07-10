import XCTest
@testable import BHTerminal

final class SessionImportTests: XCTestCase {

    // MARK: - MobaXterm

    /// Synthetic sample in MobaXterm's real `[Bookmarks]` shape:
    /// `Name= #<type>#<flag>%host%port%user%…`, folders via `SubRep`.
    private let mobaSample = """
    [Bookmarks]
    SubRep=
    ImgNum=42
    web1= #109#0%192.168.1.10%22%root%%-1%-1%%%22%%0%0%0%%%-1%0%0%0%
    RDP box= #91#4%10.0.0.9%3389%admin%
    [Bookmarks_1]
    SubRep=Production\\\\Web
    ImgNum=41
    api= #109#0%api.example.com%2222%deploy%%-1%-1%
    desktop= #97#0%10.0.0.50%5900%%
    """

    func testMobaXtermParsesSSHFolderAndVNC() {
        let (sessions, warnings) = MobaXtermImporter.parse(mobaSample)

        // web1 (root SSH), api (nested SSH), desktop (nested VNC). RDP skipped.
        XCTAssertEqual(sessions.count, 3)

        let web1 = sessions.first { $0.name == "web1" }
        XCTAssertEqual(web1?.hostname, "192.168.1.10")
        XCTAssertEqual(web1?.port, 22)
        XCTAssertEqual(web1?.username, "root")
        XCTAssertEqual(web1?.connectionType, .ssh)
        XCTAssertEqual(web1?.folderPath, [])

        let api = sessions.first { $0.name == "api" }
        XCTAssertEqual(api?.hostname, "api.example.com")
        XCTAssertEqual(api?.port, 2222)
        XCTAssertEqual(api?.username, "deploy")
        XCTAssertEqual(api?.folderPath, ["Production", "Web"])

        let desktop = sessions.first { $0.name == "desktop" }
        XCTAssertEqual(desktop?.connectionType, .vnc)
        XCTAssertEqual(desktop?.hostname, "10.0.0.50")
        XCTAssertEqual(desktop?.port, 5900)

        // RDP (type 91) reported, not silently dropped.
        XCTAssertTrue(warnings.contains { $0.contains("RDP box") && $0.contains("91") })
    }

    func testDetectionRoutesToMobaXterm() throws {
        let (source, sessions, _) = try SessionImport.parse(contents: mobaSample, filename: "export.mxtsessions")
        XCTAssertEqual(source, .mobaXterm)
        XCTAssertEqual(sessions.count, 3)
    }

    func testEncryptedMobaConfThrowsGuidance() {
        // No [Bookmarks] + a MobaXterm extension = the encrypted blob.
        let blob = "_@nawdfVxvKFbQ4Vxvabcdef0123456789=="
        XCTAssertThrowsError(try SessionImport.parse(contents: blob, filename: "config.mobaconf")) { error in
            guard case SessionImportError.encryptedMobaConf = error else {
                return XCTFail("Expected encryptedMobaConf, got \(error)")
            }
        }
    }

    // MARK: - SSH config

    func testSSHConfigParsing() {
        let cfg = """
        # my hosts
        Host prod
            HostName 203.0.113.5
            User deploy
            Port 2200

        Host *.internal
            User root

        Host gw
            HostName gw.example.com
        """
        let (sessions, _) = SSHConfigImporter.parse(cfg)
        // "prod" and "gw" — the "*.internal" wildcard is skipped.
        XCTAssertEqual(sessions.count, 2)

        let prod = sessions.first { $0.name == "prod" }
        XCTAssertEqual(prod?.hostname, "203.0.113.5")
        XCTAssertEqual(prod?.username, "deploy")
        XCTAssertEqual(prod?.port, 2200)

        let gw = sessions.first { $0.name == "gw" }
        XCTAssertEqual(gw?.hostname, "gw.example.com")
        XCTAssertEqual(gw?.port, 22)          // default
        XCTAssertEqual(gw?.username, "")       // committer fills NSUserName()
    }

    // MARK: - Commit

    func testCommitBuildsFoldersAndSkipsUnsafe() {
        let store = SessionStore(fileURL: Self.tempStoreURL())
        let sessions = [
            ParsedSession(name: "good", hostname: "example.com", port: 22, username: "root", connectionType: .ssh, folderPath: ["A", "B"]),
            ParsedSession(name: "evil", hostname: "-oProxyCommand=id", port: 22, username: "root", connectionType: .ssh, folderPath: []),
        ]
        let result = SessionImportCommitter.commit(source: .mobaXterm, sessions: sessions, warnings: [], into: store)

        XCTAssertEqual(result.importedHosts, 1)      // "evil" rejected by SSHSafety
        XCTAssertEqual(result.skipped, 1)
        // root import folder + A + B
        XCTAssertEqual(result.importedFolders, 3)
        XCTAssertTrue(store.hosts.contains { $0.name == "good" })
        XCTAssertFalse(store.hosts.contains { $0.hostname.hasPrefix("-") })
    }

    // MARK: - MobaXterm stored-passwords dump

    private let mobaPasswords = """
    ssh22:root@vps.example.com = secretA\r
    root@vps.example.com = secretA\r
    ssh2250:deploy@build.example.com = secretB\r
    vnc:admin@10.0.0.5 = secretC\r
    vnc5901:admin@10.0.0.6 = secretD\r
    rdp:administrator@win.example.com = secretE\r
    lonelyuser@only-bare.example.com = secretF\r
    """

    func testMobaPasswordsParsing() {
        let (sessions, warnings) = MobaXtermPasswordsImporter.parse(mobaPasswords)

        // root@vps (ssh22, bare deduped), deploy@build (ssh2250), admin@.5 (vnc),
        // admin@.6 (vnc5901), lonelyuser (bare-only → ssh22). RDP skipped.
        XCTAssertEqual(sessions.count, 5)

        let root = sessions.first { $0.username == "root" }
        XCTAssertEqual(root?.hostname, "vps.example.com")
        XCTAssertEqual(root?.port, 22)
        XCTAssertEqual(root?.connectionType, .ssh)
        XCTAssertEqual(root?.password, "secretA")

        let deploy = sessions.first { $0.username == "deploy" }
        XCTAssertEqual(deploy?.port, 2250)
        XCTAssertEqual(deploy?.password, "secretB")

        let vnc = sessions.first { $0.hostname == "10.0.0.5" }
        XCTAssertEqual(vnc?.connectionType, .vnc)
        XCTAssertEqual(vnc?.port, 5900)

        let vnc2 = sessions.first { $0.hostname == "10.0.0.6" }
        XCTAssertEqual(vnc2?.port, 5901)

        let lonely = sessions.first { $0.username == "lonelyuser" }
        XCTAssertEqual(lonely?.connectionType, .ssh)
        XCTAssertEqual(lonely?.port, 22)

        // bare root@vps didn't create a second host
        XCTAssertEqual(sessions.filter { $0.hostname == "vps.example.com" }.count, 1)
        XCTAssertTrue(warnings.contains { $0.contains("SSH and VNC") })
    }

    func testPasswordsDetectionAndCommitStoresSecret() throws {
        let (source, sessions, _) = try SessionImport.parse(contents: mobaPasswords, filename: "MobaXterm Stored Passwords.txt")
        XCTAssertEqual(source, .mobaXtermPasswords)

        let store = SessionStore(fileURL: Self.tempStoreURL())
        let result = SessionImportCommitter.commit(source: source, sessions: sessions, warnings: [], into: store)
        XCTAssertEqual(result.importedHosts, 5)

        // A committed host uses password auth and its secret is in the Keychain.
        guard let root = store.hosts.first(where: { $0.username == "root" }) else {
            return XCTFail("root host missing")
        }
        XCTAssertEqual(root.authMethod, .password)
        let stored = try KeychainService.read(account: root.keychainAccount)
        XCTAssertEqual(stored, "secretA")
        try? KeychainService.delete(account: root.keychainAccount) // cleanup
        for host in store.hosts { try? KeychainService.delete(account: host.keychainAccount) }
    }

    private static func tempStoreURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("bhterminal-import-test-\(UUID().uuidString).json")
    }
}
