import AppKit
import Observation

/// App-wide UI preferences (theme/font) — deliberately separate from
/// SessionStore, which only ever holds per-host data. Backed by
/// UserDefaults directly rather than @AppStorage since it also needs to be
/// read from non-View contexts (SplitPaneView's NSViewRepresentable
/// Coordinator, when creating a new PTYSession).
@Observable
final class AppSettings {
    static let shared = AppSettings()

    static let monospaceFontNames = ["Menlo", "Monaco", "Courier New", "SF Mono"]

    var theme: AppTheme {
        didSet { UserDefaults.standard.set(theme.rawValue, forKey: Keys.theme) }
    }
    var fontName: String {
        didSet { UserDefaults.standard.set(fontName, forKey: Keys.fontName) }
    }
    var fontSize: Double {
        didSet { UserDefaults.standard.set(fontSize, forKey: Keys.fontSize) }
    }

    private enum Keys {
        static let theme = "bhterminal.theme"
        static let fontName = "bhterminal.fontName"
        static let fontSize = "bhterminal.fontSize"
    }

    private init() {
        let defaults = UserDefaults.standard
        theme = AppTheme(rawValue: defaults.string(forKey: Keys.theme) ?? "") ?? .system
        fontName = defaults.string(forKey: Keys.fontName) ?? "Menlo"
        let storedSize = defaults.object(forKey: Keys.fontSize) as? Double
        fontSize = storedSize ?? 13
    }

    var font: NSFont {
        NSFont(name: fontName, size: fontSize) ?? NSFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
    }
}
