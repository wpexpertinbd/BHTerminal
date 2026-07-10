import Foundation

/// Parses MobaXterm's "stored passwords" text dump — one credential per line:
///
///     ssh22:root@vps.example.com = «password»
///     vnc:admin@10.0.0.5 = «password»
///     root@vps.example.com = «password»      (bare "credential", no protocol)
///
/// The key is `[<proto><port>:]<user>@<host>`; everything after the first
/// ` = ` is the password. `ssh<port>` → SSH on that port, `vnc[<port>]` → VNC
/// (default 5900), other protocols (rdp/telnet/…) are counted and reported.
/// Bare `user@host` lines duplicate a session's password, so they only create
/// a host (SSH:22) when no typed entry already covers that user@host.
///
/// This is the one importer that carries passwords — the committer routes them
/// into the Keychain.
enum MobaXtermPasswordsImporter {
    static func parse(_ contents: String) -> (sessions: [ParsedSession], warnings: [String]) {
        var byKey: [String: ParsedSession] = [:]
        var order: [String] = []
        var unsupported = 0

        let lines = contents.components(separatedBy: .newlines)

        // Pass 1 — typed entries (they carry the real protocol + port).
        for line in lines {
            guard let (key, password) = splitKeyValue(line) else { continue }
            guard let parsed = parseTypedKey(key) else { continue }
            guard let type = parsed.type else { unsupported += 1; continue }

            let mapKey = "\(type)|\(parsed.host)|\(parsed.user)"
            if byKey[mapKey] == nil {
                byKey[mapKey] = ParsedSession(
                    name: "\(parsed.user)@\(parsed.host)",
                    hostname: parsed.host,
                    port: parsed.port,
                    username: parsed.user,
                    connectionType: type,
                    folderPath: [],
                    password: password.isEmpty ? nil : password
                )
                order.append(mapKey)
            }
        }

        // Pass 2 — bare user@host, only if no typed SSH entry covers it.
        for line in lines {
            guard let (key, password) = splitKeyValue(line) else { continue }
            guard parseTypedKey(key) == nil, let (user, host) = parseBareKey(key) else { continue }

            let mapKey = "\(Host.ConnectionType.ssh)|\(host)|\(user)"
            if byKey[mapKey] == nil {
                byKey[mapKey] = ParsedSession(
                    name: "\(user)@\(host)",
                    hostname: host,
                    port: 22,
                    username: user,
                    connectionType: .ssh,
                    folderPath: [],
                    password: password.isEmpty ? nil : password
                )
                order.append(mapKey)
            }
        }

        var warnings: [String] = []
        if unsupported > 0 {
            warnings.append("Skipped \(unsupported) entr\(unsupported == 1 ? "y" : "ies") for protocols BHTerminal doesn't support (only SSH and VNC are imported).")
        }
        return (order.compactMap { byKey[$0] }, warnings)
    }

    // MARK: - Line parsing

    /// Splits on the first " = "; the value (password) is the whole remainder.
    private static func splitKeyValue(_ line: String) -> (key: String, value: String)? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard let range = trimmed.range(of: " = ") else { return nil }
        let key = String(trimmed[..<range.lowerBound]).trimmingCharacters(in: .whitespaces)
        let value = String(trimmed[range.upperBound...])
        return key.isEmpty ? nil : (key, value)
    }

    private struct TypedKey {
        var type: Host.ConnectionType?   // nil = recognized protocol we don't support
        var port: Int
        var user: String
        var host: String
    }

    /// `proto[port]:user@host` → components; nil if there's no `proto:` prefix.
    private static func parseTypedKey(_ key: String) -> TypedKey? {
        guard let colon = key.firstIndex(of: ":") else { return nil }
        let prefix = String(key[..<colon])
        let rest = String(key[key.index(after: colon)...])

        // prefix = letters + optional trailing digits (e.g. "ssh2250", "vnc").
        let letters = prefix.prefix { $0.isLetter }
        let digits = prefix.dropFirst(letters.count)
        guard !letters.isEmpty, digits.allSatisfy(\.isNumber) else { return nil }

        guard let (user, host) = parseBareKey(rest) else { return nil }
        let portValue = Int(digits)

        switch letters.lowercased() {
        case "ssh", "sftp":
            return TypedKey(type: .ssh, port: portValue ?? 22, user: user, host: host)
        case "vnc":
            return TypedKey(type: .vnc, port: portValue ?? 5900, user: user, host: host)
        default:
            return TypedKey(type: nil, port: portValue ?? 0, user: user, host: host) // rdp/telnet/ftp/…
        }
    }

    /// `user@host` split on the LAST `@` (usernames rarely contain `@`, but
    /// hosts never do, so the last one is the separator).
    private static func parseBareKey(_ key: String) -> (user: String, host: String)? {
        guard let at = key.lastIndex(of: "@") else { return nil }
        let user = String(key[..<at])
        let host = String(key[key.index(after: at)...])
        guard !user.isEmpty, !host.isEmpty else { return nil }
        return (user, host)
    }
}
