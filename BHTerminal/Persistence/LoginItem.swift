import Foundation
import ServiceManagement

/// Launch-at-login, backed by SMAppService (macOS 13+). Registering adds the
/// current app bundle as a login item; the OS re-launches it at sign-in.
enum LoginItem {
    static var isEnabled: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// Throws if the OS refuses to (un)register — the caller reverts the UI
    /// toggle to whatever `isEnabled` reports afterward.
    static func setEnabled(_ enabled: Bool) throws {
        switch (enabled, SMAppService.mainApp.status) {
        case (true, let status) where status != .enabled:
            try SMAppService.mainApp.register()
        case (false, .enabled):
            try SMAppService.mainApp.unregister()
        default:
            break // already in the requested state
        }
    }
}
