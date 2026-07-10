import XCTest
@testable import BHTerminal

/// Guards against SessionStore.load()'s `try?` silently dropping every
/// saved host when a JSON field is added after real sessions.json files
/// already exist on disk (connectionType was added post-hoc).
final class HostBackwardCompatTests: XCTestCase {
    func testDecodingHostWithoutConnectionTypeDefaultsToSSH() throws {
        let legacyJSON = """
        {
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "Legacy Host",
            "hostname": "example.com",
            "port": 22,
            "username": "root",
            "authMethod": { "agent": {} },
            "tunnels": [],
            "sortOrder": 0,
            "notes": ""
        }
        """
        let host = try JSONDecoder().decode(Host.self, from: Data(legacyJSON.utf8))
        XCTAssertEqual(host.connectionType, .ssh)
        XCTAssertEqual(host.name, "Legacy Host")
    }

    func testFullDocumentWithoutConnectionTypeStillDecodesAllHosts() throws {
        let legacyDocumentJSON = """
        {
            "folders": [],
            "hosts": [
                {
                    "id": "00000000-0000-0000-0000-000000000002",
                    "name": "Prod",
                    "hostname": "prod.example.com",
                    "port": 22,
                    "username": "deploy",
                    "authMethod": { "agent": {} },
                    "tunnels": [],
                    "sortOrder": 0,
                    "notes": ""
                }
            ],
            "snippets": []
        }
        """
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("bhterminal-legacy-\(UUID().uuidString).json")
        try Data(legacyDocumentJSON.utf8).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        let store = SessionStore(fileURL: url)
        XCTAssertEqual(store.hosts.count, 1)
        XCTAssertEqual(store.hosts.first?.name, "Prod")
        XCTAssertEqual(store.hosts.first?.connectionType, .ssh)
    }
}
