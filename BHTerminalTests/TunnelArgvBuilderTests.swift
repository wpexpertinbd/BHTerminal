import XCTest
@testable import BHTerminal

final class TunnelArgvBuilderTests: XCTestCase {
    private let host = Host(name: "Prod", hostname: "example.com", username: "root", authMethod: .agent)

    func testLocalForward() throws {
        let tunnel = TunnelRule(kind: .local, listenPort: 8080, destHost: "127.0.0.1", destPort: 80)
        let (executable, args) = try TunnelArgvBuilder.build(for: tunnel, host: host)
        XCTAssertEqual(executable, "/usr/bin/ssh")
        XCTAssertEqual(args, ["-N", "-L", "127.0.0.1:8080:127.0.0.1:80", "--", "root@example.com"])
    }

    func testRemoteForward() throws {
        let tunnel = TunnelRule(kind: .remote, listenPort: 9000, destHost: "localhost", destPort: 3000)
        let (_, args) = try TunnelArgvBuilder.build(for: tunnel, host: host)
        XCTAssertEqual(args, ["-N", "-R", "127.0.0.1:9000:localhost:3000", "--", "root@example.com"])
    }

    func testDynamicSOCKS() throws {
        let tunnel = TunnelRule(kind: .dynamic, listenPort: 1080)
        let (_, args) = try TunnelArgvBuilder.build(for: tunnel, host: host)
        XCTAssertEqual(args, ["-N", "-D", "127.0.0.1:1080", "--", "root@example.com"])
    }

    func testCustomPortHost() throws {
        let customPortHost = Host(name: "Custom", hostname: "example.com", port: 2222, username: "deploy", authMethod: .agent)
        let tunnel = TunnelRule(kind: .local, listenPort: 8080, destHost: "127.0.0.1", destPort: 80)
        let (_, args) = try TunnelArgvBuilder.build(for: tunnel, host: customPortHost)
        XCTAssertEqual(args, ["-N", "-p", "2222", "-L", "127.0.0.1:8080:127.0.0.1:80", "--", "deploy@example.com"])
    }

    func testJumpHost() throws {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", username: "jump")
        let target = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let tunnel = TunnelRule(kind: .local, listenPort: 5432, destHost: "127.0.0.1", destPort: 5432)
        let (_, args) = try TunnelArgvBuilder.build(for: tunnel, host: target) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(args, ["-N", "-J", "jump@bastion.example.com", "-L", "127.0.0.1:5432:127.0.0.1:5432", "--", "root@10.0.0.5"])
    }

    func testPrivateKeyAuth() throws {
        let keyHost = Host(name: "Key", hostname: "example.com", username: "ben", authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        let tunnel = TunnelRule(kind: .local, listenPort: 8080, destHost: "127.0.0.1", destPort: 80)
        let (_, args) = try TunnelArgvBuilder.build(for: tunnel, host: keyHost)
        XCTAssertEqual(args, ["-N", "-i", "\(NSHomeDirectory())/.ssh/id_ed25519", "-L", "127.0.0.1:8080:127.0.0.1:80", "--", "ben@example.com"])
    }
}
