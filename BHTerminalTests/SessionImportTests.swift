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

    private static func tempStoreURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("bhterminal-import-test-\(UUID().uuidString).json")
    }
}
