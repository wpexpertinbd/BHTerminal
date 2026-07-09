import XCTest
@testable import BHTerminal

final class SSHArgvBuilderTests: XCTestCase {
    func testDefaultPortAgentAuth() {
        let host = Host(name: "Prod", hostname: "example.com", username: "root", authMethod: .agent)
        let (executable, args) = SSHArgvBuilder.build(for: host)
        XCTAssertEqual(executable, "/usr/bin/ssh")
        XCTAssertEqual(args, ["root@example.com"])
    }

    func testCustomPort() {
        let host = Host(name: "Custom", hostname: "example.com", port: 2222, username: "deploy", authMethod: .agent)
        let (_, args) = SSHArgvBuilder.build(for: host)
        XCTAssertEqual(args, ["-p", "2222", "deploy@example.com"])
    }

    func testPrivateKeyExpandsTilde() {
        let host = Host(name: "Key", hostname: "example.com", username: "ben",
                         authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        let (_, args) = SSHArgvBuilder.build(for: host)
        let home = NSHomeDirectory()
        XCTAssertEqual(args, ["-i", "\(home)/.ssh/id_ed25519", "ben@example.com"])
    }

    func testJumpHostDefaultPort() {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", username: "jump")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let (_, args) = SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(args, ["-J", "jump@bastion.example.com", "root@10.0.0.5"])
    }

    func testJumpHostCustomPort() {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", port: 2200, username: "jump")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let (_, args) = SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(args, ["-J", "jump@bastion.example.com:2200", "root@10.0.0.5"])
    }

    func testUnresolvableJumpHostIsOmitted() {
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: UUID())
        let (_, args) = SSHArgvBuilder.build(for: host)
        XCTAssertEqual(args, ["root@10.0.0.5"])
    }
}
