import XCTest
@testable import BHTerminal

final class SessionStoreTests: XCTestCase {
    private func makeTempStore() -> SessionStore {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("bhterminal-test-\(UUID().uuidString).json")
        return SessionStore(fileURL: url)
    }

    func testAddAndPersistHostRoundTrip() {
        let store = makeTempStore()
        defer { try? FileManager.default.removeItem(at: store.fileURL) }

        let host = Host(name: "Test VPS", hostname: "example.com", port: 2222, username: "root", authMethod: .agent)
        store.addHost(host)

        XCTAssertTrue(FileManager.default.fileExists(atPath: store.fileURL.path))

        let reloaded = SessionStore(fileURL: store.fileURL)
        XCTAssertEqual(reloaded.hosts.count, 1)
        XCTAssertEqual(reloaded.hosts.first?.name, "Test VPS")
        XCTAssertEqual(reloaded.hosts.first?.hostname, "example.com")
        XCTAssertEqual(reloaded.hosts.first?.port, 2222)
        XCTAssertEqual(reloaded.hosts.first?.authMethod, .agent)
    }

    func testPrivateKeyAuthMethodRoundTrips() {
        let store = makeTempStore()
        defer { try? FileManager.default.removeItem(at: store.fileURL) }

        let host = Host(name: "Key Auth", hostname: "host.example.com", username: "ben",
                         authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        store.addHost(host)

        let reloaded = SessionStore(fileURL: store.fileURL)
        XCTAssertEqual(reloaded.hosts.first?.authMethod, .privateKey(path: "~/.ssh/id_ed25519"))
    }

    func testFolderCascadeDeleteRemovesNestedHosts() {
        let store = makeTempStore()
        defer { try? FileManager.default.removeItem(at: store.fileURL) }

        let folder = store.addFolder(HostFolder(name: "Production"))
        let host = store.addHost(Host(name: "Web1", hostname: "web1.example.com", username: "deploy", folderID: folder.id))

        XCTAssertEqual(store.hosts.count, 1)
        store.deleteFolder(folder.id)

        XCTAssertTrue(store.folders.isEmpty)
        XCTAssertFalse(store.hosts.contains { $0.id == host.id })
    }

    func testTreeBuildsNestedStructure() {
        let store = makeTempStore()
        defer { try? FileManager.default.removeItem(at: store.fileURL) }

        let parent = store.addFolder(HostFolder(name: "Clients"))
        let child = store.addFolder(HostFolder(name: "BiswasHost", parentFolderID: parent.id))
        store.addHost(Host(name: "Main VPS", hostname: "biswashost.com", username: "root", folderID: child.id))

        let tree = store.tree
        XCTAssertEqual(tree.count, 1)
        guard case .folder(let topFolder, let topChildren) = tree[0] else {
            return XCTFail("Expected top-level folder")
        }
        XCTAssertEqual(topFolder.name, "Clients")
        XCTAssertEqual(topChildren.count, 1)

        guard case .folder(let nested, let nestedChildren) = topChildren[0] else {
            return XCTFail("Expected nested folder")
        }
        XCTAssertEqual(nested.name, "BiswasHost")
        XCTAssertEqual(nestedChildren.count, 1)

        guard case .host(let leafHost) = nestedChildren[0] else {
            return XCTFail("Expected host leaf")
        }
        XCTAssertEqual(leafHost.name, "Main VPS")
    }

    func testDeleteHostRemovesItFromStore() {
        let store = makeTempStore()
        defer { try? FileManager.default.removeItem(at: store.fileURL) }

        let host = store.addHost(Host(name: "Temp", hostname: "temp.example.com", username: "root"))
        store.deleteHost(host.id)

        XCTAssertTrue(store.hosts.isEmpty)
    }
}
