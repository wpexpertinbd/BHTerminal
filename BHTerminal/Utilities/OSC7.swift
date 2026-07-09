import Foundation

/// Parses the payload SwiftTerm hands back from an OSC 7 "current directory"
/// escape sequence. SwiftTerm passes the raw string after `ESC ]7;` through
/// unmodified (see Terminal.swift's `oscSetCurrentDirectory`), so this
/// still needs to strip the `file://host` prefix per the OSC 7 convention
/// (used by iTerm2/VSCode/etc., so any pre-existing shell integration the
/// user already has works here too, not just PTYSession's own hook).
enum OSC7 {
    static func parsePath(from raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        if let url = URL(string: trimmed), url.scheme == "file" {
            return url.path.isEmpty ? nil : url.path
        }

        guard trimmed.hasPrefix("file://") else {
            return trimmed.hasPrefix("/") ? trimmed : nil
        }
        let afterScheme = trimmed.dropFirst("file://".count)
        guard let slashIndex = afterScheme.firstIndex(of: "/") else { return nil }
        return String(afterScheme[slashIndex...])
    }
}
