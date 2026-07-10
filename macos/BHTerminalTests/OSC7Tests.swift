import XCTest
@testable import BHTerminal

final class OSC7Tests: XCTestCase {
    func testStandardFileURIWithHost() {
        XCTAssertEqual(OSC7.parsePath(from: "file://myhost/home/ben/projects"), "/home/ben/projects")
    }

    func testFileURIWithNoHost() {
        XCTAssertEqual(OSC7.parsePath(from: "file:///home/ben"), "/home/ben")
    }

    func testPercentEncodedPath() {
        XCTAssertEqual(OSC7.parsePath(from: "file://myhost/home/ben/My%20Files"), "/home/ben/My Files")
    }

    func testRawAbsolutePathFallback() {
        XCTAssertEqual(OSC7.parsePath(from: "/var/www/site"), "/var/www/site")
    }

    func testEmptyStringReturnsNil() {
        XCTAssertNil(OSC7.parsePath(from: ""))
        XCTAssertNil(OSC7.parsePath(from: "   "))
    }

    func testGarbageReturnsNil() {
        XCTAssertNil(OSC7.parsePath(from: "not-a-path-or-uri"))
    }

    func testTrimsWhitespace() {
        XCTAssertEqual(OSC7.parsePath(from: "  /home/ben  "), "/home/ben")
    }
}
