import XCTest
@testable import BHTerminal

/// The updater decides whether to offer a download by comparing version
/// strings, and only ever installs from a GitHub-hosted .pkg.
final class UpdateCheckerTests: XCTestCase {

    func testDetectsNewerVersions() {
        XCTAssertTrue(UpdateChecker.isNewer("1.2.1", than: "1.2.0"))
        XCTAssertTrue(UpdateChecker.isNewer("1.3.0", than: "1.2.9"))
        XCTAssertTrue(UpdateChecker.isNewer("2.0.0", than: "1.9.9"))
        // Numeric, not lexicographic — the classic "1.10 < 1.9" bug.
        XCTAssertTrue(UpdateChecker.isNewer("1.10.0", than: "1.9.3"))
        XCTAssertTrue(UpdateChecker.isNewer("1.2.10", than: "1.2.9"))
        // Shorter strings are zero-padded.
        XCTAssertTrue(UpdateChecker.isNewer("1.3", than: "1.2.9"))
        XCTAssertTrue(UpdateChecker.isNewer("1.2.0.1", than: "1.2.0"))
    }

    func testDoesNotOfferSameOrOlderVersions() {
        XCTAssertFalse(UpdateChecker.isNewer("1.2.0", than: "1.2.0"))
        XCTAssertFalse(UpdateChecker.isNewer("1.2.0", than: "1.2.1"))
        XCTAssertFalse(UpdateChecker.isNewer("1.9.3", than: "1.10.0"))
        // Equivalent with trailing zeros.
        XCTAssertFalse(UpdateChecker.isNewer("1.2", than: "1.2.0"))
        XCTAssertFalse(UpdateChecker.isNewer("1.2.0", than: "1.2"))
    }

    /// A tampered or unexpected API response must not be able to point the
    /// updater at an arbitrary download.
    func testOnlyTrustsGitHubHTTPSAssets() {
        let trusted = [
            "https://github.com/wpexpertinbd/BHTerminal/releases/download/v1.2.0/BHTerminal-1.2.0.pkg",
            "https://objects.githubusercontent.com/github-production-release-asset/x/y.pkg"
        ]
        for url in trusted {
            XCTAssertTrue(UpdateChecker.isTrustedAssetURL(url), url)
        }

        let rejected = [
            // Plain HTTP, even on GitHub.
            "http://github.com/wpexpertinbd/BHTerminal/releases/download/v1/x.pkg",
            // Someone else's host.
            "https://evil.example.com/BHTerminal.pkg",
            // Host that merely CONTAINS github.com.
            "https://github.com.evil.example.com/x.pkg",
            "https://notgithub.com/x.pkg",
            // Non-web schemes.
            "file:///tmp/evil.pkg",
            "ftp://github.com/x.pkg",
            ""
        ]
        for url in rejected {
            XCTAssertFalse(UpdateChecker.isTrustedAssetURL(url), url)
        }
    }
}
