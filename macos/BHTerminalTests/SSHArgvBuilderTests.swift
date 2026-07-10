import XCTest
@testable import BHTerminal

final class SSHArgvBuilderTests: XCTestCase {
    func testDefaultPortAgentAuth() throws {
        let host = Host(name: "Prod", hostname: "example.com", username: "root", authMethod: .agent)
        let (executable, args) = try SSHArgvBuilder.build(for: host)
        XCTAssertEqual(executable, "/usr/bin/ssh")
        XCTAssertEqual(args, ["--", "root@example.com"])
    }

    func testCustomPort() throws {
        let host = Host(name: "Custom", hostname: "example.com", port: 2222, username: "deploy", authMethod: .agent)
        let (_, args) = try SSHArgvBuilder.build(for: host)
        XCTAssertEqual(args, ["-p", "2222", "--", "deploy@example.com"])
    }

    func testPrivateKeyExpandsTilde() throws {
        let host = Host(name: "Key", hostname: "example.com", username: "ben",
                         authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        let (_, args) = try SSHArgvBuilder.build(for: host)
        let home = NSHomeDirectory()
        XCTAssertEqual(args, ["-i", "\(home)/.ssh/id_ed25519", "--", "ben@example.com"])
    }

    func testJumpHostDefaultPort() throws {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", username: "jump")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let (_, args) = try SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(args, ["-J", "jump@bastion.example.com", "--", "root@10.0.0.5"])
    }

    func testJumpHostCustomPort() throws {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", port: 2200, username: "jump")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let (_, args) = try SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(args, ["-J", "jump@bastion.example.com:2200", "--", "root@10.0.0.5"])
    }

    func testUnresolvableJumpHostIsOmitted() throws {
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: UUID())
        let (_, args) = try SSHArgvBuilder.build(for: host)
        XCTAssertEqual(args, ["--", "root@10.0.0.5"])
    }

    // MARK: - Argument-injection guards

    func testProxyCommandInjectionViaUsernameIsRejected() {
        let host = Host(name: "Evil", hostname: "example.com", username: "-oProxyCommand=touch /tmp/pwned", authMethod: .agent)
        XCTAssertThrowsError(try SSHArgvBuilder.build(for: host)) { error in
            guard case SSHSafetyError.unsafeUsername = error else {
                return XCTFail("Expected unsafeUsername, got \(error)")
            }
        }
    }

    func testOptionLikeHostnameIsRejected() {
        let host = Host(name: "Evil", hostname: "-oProxyCommand=id", username: "root", authMethod: .agent)
        XCTAssertThrowsError(try SSHArgvBuilder.build(for: host)) { error in
            guard case SSHSafetyError.unsafeHostname = error else {
                return XCTFail("Expected unsafeHostname, got \(error)")
            }
        }
    }

    func testUnsafeJumpHostIsRejected() {
        let jump = Host(name: "EvilJump", hostname: "bastion.example.com", username: "-oProxyCommand=id")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        XCTAssertThrowsError(try SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil })
    }

    func testWhitespaceAndShellMetacharsRejected() {
        for bad in ["ex ample.com", "example.com;id", "a$(id)b", "host`id`", "h|id"] {
            XCTAssertFalse(SSHSafety.isValidHostname(bad), "\(bad) should be rejected")
        }
        XCTAssertTrue(SSHSafety.isValidHostname("example.com"))
        XCTAssertTrue(SSHSafety.isValidHostname("10.0.0.5"))
        XCTAssertTrue(SSHSafety.isValidHostname("[fe80::1%en0]"))
        XCTAssertTrue(SSHSafety.isValidUsername("root"))
        XCTAssertTrue(SSHSafety.isValidUsername("deploy_user-1.svc"))
    }
}
