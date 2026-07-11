import XCTest
@testable import BHTerminal

final class SSHArgvBuilderTests: XCTestCase {

    // MARK: - Terminal argv

    func testTerminalEndsWithDestinationAndHasControlMaster() throws {
        let host = Host(name: "Prod", hostname: "example.com", username: "root", authMethod: .agent)
        let (executable, args) = try SSHArgvBuilder.build(for: host)
        XCTAssertEqual(executable, "/usr/bin/ssh")
        // "--" then the destination is always the tail.
        XCTAssertEqual(Array(args.suffix(2)), ["--", "root@example.com"])
        // Multiplexing master so the SFTP pane can reuse the connection.
        XCTAssertTrue(args.contains("ControlMaster=auto"))
        XCTAssertTrue(args.contains("ControlPath=/tmp/bht-%C"))
        XCTAssertTrue(args.contains("ControlPersist=120"))
    }

    func testTerminalCustomPort() throws {
        let host = Host(name: "Custom", hostname: "example.com", port: 2222, username: "deploy", authMethod: .agent)
        let (_, args) = try SSHArgvBuilder.build(for: host)
        XCTAssertEqual(Array(args.prefix(2)), ["-p", "2222"])
        XCTAssertEqual(Array(args.suffix(2)), ["--", "deploy@example.com"])
    }

    func testTerminalPrivateKeyExpandsTilde() throws {
        let host = Host(name: "Key", hostname: "example.com", username: "ben",
                         authMethod: .privateKey(path: "~/.ssh/id_ed25519"))
        let (_, args) = try SSHArgvBuilder.build(for: host)
        let home = NSHomeDirectory()
        XCTAssertEqual(Array(args.prefix(2)), ["-i", "\(home)/.ssh/id_ed25519"])
        XCTAssertEqual(Array(args.suffix(2)), ["--", "ben@example.com"])
    }

    func testTerminalJumpHost() throws {
        let jump = Host(name: "Bastion", hostname: "bastion.example.com", port: 2200, username: "jump")
        let host = Host(name: "Internal", hostname: "10.0.0.5", username: "root", jumpHostID: jump.id)
        let (_, args) = try SSHArgvBuilder.build(for: host) { id in id == jump.id ? jump : nil }
        XCTAssertEqual(Array(args.prefix(2)), ["-J", "jump@bastion.example.com:2200"])
        XCTAssertEqual(Array(args.suffix(2)), ["--", "root@10.0.0.5"])
    }

    // MARK: - SFTP-subsystem argv

    func testSFTPArgvShape() throws {
        let host = Host(name: "Prod", hostname: "example.com", port: 2222, username: "root", authMethod: .agent)
        let (executable, args) = try SSHArgvBuilder.buildSFTP(for: host)
        XCTAssertEqual(executable, "/usr/bin/ssh")
        // Requests the sftp subsystem on the destination, options terminated.
        XCTAssertEqual(Array(args.suffix(4)), ["-s", "--", "root@example.com", "sftp"])
        XCTAssertEqual(Array(args.prefix(2)), ["-p", "2222"])
        // Reuses the terminal's master and never blocks on a prompt.
        XCTAssertTrue(args.contains("ControlPath=/tmp/bht-%C"))
        XCTAssertTrue(args.contains("BatchMode=yes"))
        // SFTP itself must not become the master.
        XCTAssertFalse(args.contains("ControlMaster=auto"))
    }

    func testSFTPRejectsUnsafeHost() {
        let host = Host(name: "Evil", hostname: "-oProxyCommand=id", username: "root", authMethod: .agent)
        XCTAssertThrowsError(try SSHArgvBuilder.buildSFTP(for: host)) { error in
            guard case SSHSafetyError.unsafeHostname = error else {
                return XCTFail("Expected unsafeHostname, got \(error)")
            }
        }
    }

    // MARK: - Argument-injection guards (terminal)

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
