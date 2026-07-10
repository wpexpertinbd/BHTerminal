import Foundation

/// Parses an OpenSSH client config (`~/.ssh/config`) into sessions — a handy
/// path for anyone migrating from any tool, since MobaXterm/Termius users
/// often keep one. Reads `Host`, `HostName`, `Port`, and `User`; wildcard
/// (`*`/`?`) Host patterns are skipped (they're rules, not connectable hosts).
enum SSHConfigImporter {
    static func parse(_ contents: String) -> (sessions: [ParsedSession], warnings: [String]) {
        var sessions: [ParsedSession] = []
        let warnings: [String] = []

        var aliases: [String] = []
        var hostName = ""
        var user = ""
        var port = 22

        func flush() {
            for alias in aliases where !alias.contains("*") && !alias.contains("?") {
                sessions.append(ParsedSession(
                    name: alias,
                    hostname: hostName.isEmpty ? alias : hostName,
                    port: port,
                    username: user,
                    connectionType: .ssh,
                    folderPath: []
                ))
            }
        }

        for rawLine in contents.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty, !line.hasPrefix("#"), let (keyword, value) = keyValue(line) else { continue }

            switch keyword.lowercased() {
            case "host":
                flush()
                aliases = value.split(whereSeparator: { $0 == " " || $0 == "\t" }).map(String.init)
                hostName = ""
                user = ""
                port = 22
            case "hostname":
                hostName = value
            case "user":
                user = value
            case "port":
                port = Int(value) ?? 22
            default:
                break
            }
        }
        flush()

        return (sessions, warnings)
    }

    /// Splits a config line into (keyword, value); the separator is the first
    /// run of whitespace or an `=` (both are valid in ssh_config).
    private static func keyValue(_ line: String) -> (String, String)? {
        guard let sepIndex = line.firstIndex(where: { $0 == " " || $0 == "\t" || $0 == "=" }) else { return nil }
        let keyword = String(line[..<sepIndex])
        let value = String(line[line.index(after: sepIndex)...])
            .trimmingCharacters(in: CharacterSet(charactersIn: " \t="))
        guard !keyword.isEmpty, !value.isEmpty else { return nil }
        return (keyword, value)
    }
}
