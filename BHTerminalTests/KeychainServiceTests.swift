import XCTest
@testable import BHTerminal

final class KeychainServiceTests: XCTestCase {
    func testSaveReadDeleteRoundTrip() throws {
        let account = "test.\(UUID().uuidString)"
        defer { try? KeychainService.delete(account: account) }

        try KeychainService.save(account: account, secret: "s3cr3t-passphrase")
        XCTAssertEqual(try KeychainService.read(account: account), "s3cr3t-passphrase")

        try KeychainService.save(account: account, secret: "updated-secret")
        XCTAssertEqual(try KeychainService.read(account: account), "updated-secret")

        try KeychainService.delete(account: account)
        XCTAssertNil(try KeychainService.read(account: account))
    }

    func testReadMissingAccountReturnsNil() throws {
        let result = try KeychainService.read(account: "nonexistent.\(UUID().uuidString)")
        XCTAssertNil(result)
    }
}
