import XCTest
import AppKit
@testable import BHTerminal

/// Verifies the terminal right-click menu's *enabled* state — the bug where
/// SwiftTerm's validateUserInterfaceItem was greying out all our custom items.
/// With autoenablesItems off + manual isEnabled, Select All / Copy All are
/// always enabled, Paste follows the clipboard, Copy follows the selection.
final class PTYSessionMenuTests: XCTestCase {
    private func rightClick() -> NSEvent {
        NSEvent.mouseEvent(with: .rightMouseDown, location: .zero, modifierFlags: [],
                           timestamp: 0, windowNumber: 0, context: nil,
                           eventNumber: 0, clickCount: 1, pressure: 1)!
    }

    @MainActor
    func testMenuEnableStates() {
        let session = PTYSession(frame: NSRect(x: 0, y: 0, width: 400, height: 300))

        // Clipboard has text → Paste enabled. No terminal selection → Copy off.
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString("some text", forType: .string)

        let menu = session.menu(for: rightClick())
        XCTAssertNotNil(menu)
        XCTAssertEqual(menu?.autoenablesItems, false)

        // These must never be disabled (the reported bug).
        XCTAssertEqual(menu?.item(withTitle: "Select All")?.isEnabled, true)
        XCTAssertEqual(menu?.item(withTitle: "Copy All")?.isEnabled, true)
        // Paste reflects the clipboard.
        XCTAssertEqual(menu?.item(withTitle: "Paste")?.isEnabled, true)
        // No selection on a fresh session → Copy is off (correctly).
        XCTAssertEqual(menu?.item(withTitle: "Copy")?.isEnabled, false)
    }

    @MainActor
    func testPasteDisabledWithEmptyClipboard() {
        let session = PTYSession(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
        NSPasteboard.general.clearContents() // no string type
        let menu = session.menu(for: rightClick())
        XCTAssertEqual(menu?.item(withTitle: "Paste")?.isEnabled, false)
        XCTAssertEqual(menu?.item(withTitle: "Copy All")?.isEnabled, true)
    }
}
