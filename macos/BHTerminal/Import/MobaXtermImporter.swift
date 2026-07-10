import Foundation

/// Parses MobaXterm's plaintext session format — the `[Bookmarks]` /
/// `[Bookmarks_N]` sections found in an exported `.mxtsessions`, an
/// unencrypted `.mobaconf`, or `MobaXterm.ini`.
///
/// Each `[Bookmarks*]` section is one folder, named by its `SubRep=` value
/// (backslash-separated for nesting). Every other `key=value` in the section
/// is a session whose value looks like `#<type>#<flag>%host%port%user%…`:
///   type 109 = SSH, 97 = VNC. Other types (RDP/telnet/serial/…) are counted
///   and reported rather than silently dropped, since BHTerminal only does
///   SSH + VNC. Passwords are never present in this format.
enum MobaXtermImporter {
    static func parse(_ contents: String) -> (sessions: [ParsedSession], warnings: [String]) {
        var sessions: [ParsedSession] = []
        var warnings: [String] = []
        var inBookmarks = false
        var currentSubRep = ""

        for rawLine in contents.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)

            if line.hasPrefix("[") && line.hasSuffix("]") {
                let section = line.dropFirst().dropLast()
                inBookmarks = section.hasPrefix("Bookmarks")
                currentSubRep = ""
                continue
            }

            guard inBookmarks, !line.isEmpty, let eq = line.firstIndex(of: "=") else { continue }
            let key = String(line[..<eq]).trimmingCharacters(in: .whitespaces)
            let value = String(line[line.index(after: eq)...])

            if key == "SubRep" {
                currentSubRep = value.trimmingCharacters(in: .whitespaces)
                continue
            }
            if key == "ImgNum" { continue }

            let trimmedValue = value.trimmingCharacters(in: .whitespaces)
            guard trimmedValue.hasPrefix("#") else { continue }

            // #<type>#<rest>
            let afterFirstHash = trimmedValue.dropFirst()
            guard let secondHash = afterFirstHash.firstIndex(of: "#"),
                  let type = Int(afterFirstHash[..<secondHash]) else { continue }

            let rest = String(afterFirstHash[afterFirstHash.index(after: secondHash)...])
            let fields = rest.components(separatedBy: "%")
            func field(_ i: Int) -> String { i >= 0 && i < fields.count ? fields[i] : "" }

            let folderPath = currentSubRep.isEmpty ? [] : currentSubRep.components(separatedBy: "\\").filter { !$0.isEmpty }

            switch type {
            case 109: // SSH
                sessions.append(ParsedSession(
                    name: key,
                    hostname: field(1),
                    port: Int(field(2)) ?? 22,
                    username: field(3),
                    connectionType: .ssh,
                    folderPath: folderPath
                ))
            case 97: // VNC
                sessions.append(ParsedSession(
                    name: key,
                    hostname: field(1),
                    port: Int(field(2)) ?? 5900,
                    username: "",
                    connectionType: .vnc,
                    folderPath: folderPath
                ))
            default:
                warnings.append("Skipped “\(key)” — MobaXterm session type \(type) isn't supported (only SSH and VNC are imported).")
            }
        }

        return (sessions, warnings)
    }
}
