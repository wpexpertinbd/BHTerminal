import Foundation

enum SSHSafetyError: Error, LocalizedError {
    case unsafeHostname(String)
    case unsafeUsername(String)

    var errorDescription: String? {
        switch self {
        case .unsafeHostname(let h):
            return "“\(h)” is not a valid hostname. Hostnames may only contain letters, digits, and . - _ : % [ ] and must not start with “-”."
        case .unsafeUsername(let u):
            return "“\(u)” is not a valid username. Usernames may only contain letters, digits, and . - _ and must not start with “-”."
        }
    }
}

/// Guards against argument injection into `/usr/bin/ssh`. The ssh destination
/// operand (`user@host`) is the one argv element ssh will re-interpret as an
/// OPTION if it begins with “-” — e.g. a username of `-oProxyCommand=…` makes
/// ssh run an arbitrary command via /bin/sh before authentication (verified
/// against OpenSSH 10.x). Jump-host values ride inside `-J` and feed an inner
/// ssh, so they get the same treatment. Everything here is an allow-list:
/// anything outside the permitted character set (whitespace, shell
/// metacharacters, a leading dash) is rejected rather than escaped, because
/// for a credential-holding tool "refuse the weird value" beats "try to
/// sanitize it."
enum SSHSafety {
    private static let hostnameAllowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_:%[]")
    private static let usernameAllowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_")

    static func isValidHostname(_ s: String) -> Bool {
        !s.isEmpty && !s.hasPrefix("-") && s.allSatisfy { hostnameAllowed.contains($0) }
    }

    static func isValidUsername(_ s: String) -> Bool {
        !s.isEmpty && !s.hasPrefix("-") && s.allSatisfy { usernameAllowed.contains($0) }
    }

    /// Throws if the host — or its resolved jump host — carries a hostname or
    /// username that ssh could misinterpret as an option / shell command.
    static func validate(_ host: Host, resolveJumpHost: (UUID) -> Host? = { _ in nil }) throws {
        guard isValidHostname(host.hostname) else { throw SSHSafetyError.unsafeHostname(host.hostname) }
        guard isValidUsername(host.username) else { throw SSHSafetyError.unsafeUsername(host.username) }
        if let jumpID = host.jumpHostID, let jump = resolveJumpHost(jumpID) {
            guard isValidHostname(jump.hostname) else { throw SSHSafetyError.unsafeHostname(jump.hostname) }
            guard isValidUsername(jump.username) else { throw SSHSafetyError.unsafeUsername(jump.username) }
        }
    }
}
