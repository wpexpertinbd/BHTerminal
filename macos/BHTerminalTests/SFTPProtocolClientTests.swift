import XCTest
@testable import BHTerminal

/// Exercises the SFTP wire protocol end-to-end against macOS's own
/// `/usr/libexec/sftp-server` — the exact subsystem `ssh -s … sftp` runs on
/// the remote side — so the protocol client is validated for real, with no
/// SSH/auth needed. If the binary is missing the test skips.
final class SFTPProtocolClientTests: XCTestCase {
    private let serverPath = "/usr/libexec/sftp-server"

    func testFullFileLifecycleAgainstRealServer() async throws {
        guard FileManager.default.fileExists(atPath: serverPath) else {
            throw XCTSkip("no /usr/libexec/sftp-server on this machine")
        }

        let client = try await SFTPProtocolClient.launch(executable: serverPath, args: [])
        defer { client.close() }

        // Work inside a fresh temp dir; use the server's own realpath as the
        // base (temp dirs are symlinked on macOS, /var → /private/var).
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("bht-sftp-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let base = try await client.realpath(tmp.path)
        XCTAssertFalse(base.isEmpty)

        // write → read round-trip (also covers chunked read/write paths).
        let payload = Data((0..<70_000).map { UInt8($0 & 0xFF) }) // > one 32 KB chunk
        try await client.writeFile(base + "/big.bin", data: payload)
        let readBack = try await client.readFile(base + "/big.bin")
        XCTAssertEqual(readBack, payload)

        // small text file
        let text = Data("hello sftp\n".utf8)
        try await client.writeFile(base + "/a.txt", data: text)
        let textBack = try await client.readFile(base + "/a.txt")
        XCTAssertEqual(textBack, text)

        // listDirectory reports both, with correct file/dir typing.
        try await client.makeDirectory(base + "/sub")
        let entries = try await client.listDirectory(base)
        let names = Set(entries.map(\.filename))
        XCTAssertTrue(names.contains("a.txt"))
        XCTAssertTrue(names.contains("big.bin"))
        XCTAssertEqual(entries.first { $0.filename == "sub" }?.attributes.isDirectory, true)
        XCTAssertEqual(entries.first { $0.filename == "a.txt" }?.attributes.isDirectory, false)
        XCTAssertEqual(entries.first { $0.filename == "big.bin" }?.attributes.size, UInt64(payload.count))

        // rename + delete
        try await client.rename(from: base + "/a.txt", to: base + "/b.txt")
        let afterRename = try await client.listDirectory(base).map(\.filename)
        XCTAssertTrue(afterRename.contains("b.txt"))
        XCTAssertFalse(afterRename.contains("a.txt"))

        try await client.removeFile(base + "/b.txt")
        try await client.removeFile(base + "/big.bin")
        try await client.removeDirectory(base + "/sub")
        let empty = try await client.listDirectory(base).filter { $0.filename != "." && $0.filename != ".." }
        XCTAssertTrue(empty.isEmpty, "expected empty dir, got \(empty.map(\.filename))")
    }

    func testReadFileNotFoundThrows() async throws {
        guard FileManager.default.fileExists(atPath: serverPath) else {
            throw XCTSkip("no /usr/libexec/sftp-server on this machine")
        }
        let client = try await SFTPProtocolClient.launch(executable: serverPath, args: [])
        defer { client.close() }
        do {
            _ = try await client.readFile("/nonexistent/\(UUID().uuidString)")
            XCTFail("expected an error opening a missing file")
        } catch {
            // expected — SFTP status error
        }
    }
}
