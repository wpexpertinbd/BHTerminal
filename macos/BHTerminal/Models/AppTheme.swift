import AppKit

/// Terminal color scheme. Applied at PTYSession creation time only — a
/// preference change takes effect for newly-opened tabs/panes, not
/// retroactively on already-open ones (keeps SplitPaneView from needing to
/// track and live-restyle every session it's ever created).
enum AppTheme: String, CaseIterable, Identifiable, Codable {
    case system
    case dark
    case light

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .system: return "System"
        case .dark: return "Dark"
        case .light: return "Light"
        }
    }

    /// nil means "follow the system" — handled via configureNativeColors().
    var foreground: NSColor? {
        switch self {
        case .system: return nil
        case .dark: return NSColor(calibratedWhite: 0.90, alpha: 1)
        case .light: return NSColor(calibratedWhite: 0.10, alpha: 1)
        }
    }

    var background: NSColor? {
        switch self {
        case .system: return nil
        case .dark: return NSColor(calibratedRed: 0.08, green: 0.08, blue: 0.09, alpha: 1)
        case .light: return NSColor.white
        }
    }
}
