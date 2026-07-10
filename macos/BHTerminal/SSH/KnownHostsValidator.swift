import Foundation
import NIOSSH
import NIO
import Crypto

/// Validates the SFTP channel's host key against the user's real
/// ~/.ssh/known_hosts — the same trust store `ssh` itself uses for the
/// terminal connection. Without this, Citadel's own connection would need
/// `.acceptAnything()`, which would make the SFTP pane blindly trust
/// whatever key a server (or a MITM) presents even when the terminal
/// connection to the same host is properly verified.
///
/// A host with no matching known_hosts entry fails closed rather than
/// prompting or trust-on-first-use — connect via a terminal tab first (real
/// ssh's own TOFU prompt records it), then the SFTP pane can verify against it.
final class KnownHostsValidator: NIOSSHClientServerAuthenticationDelegate {
    private let hostname: String
    private let port: Int

    init(hostname: String, port: Int) {
        self.hostname = hostname
        self.port = port
    }

    func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
        if KnownHostsValidator.isTrusted(hostKey, hostname: hostname, port: port) {
            validationCompletePromise.succeed(())
        } else {
            validationCompletePromise.fail(SFTPConnectionError.untrustedHostKey)
        }
    }

    private static func isTrusted(_ key: NIOSSHPublicKey, hostname: String, port: Int) -> Bool {
        guard let contents = try? String(contentsOf: knownHostsURL, encoding: .utf8) else { return false }

        // OpenSSH scopes a non-standard port strictly as "[host]:port"; a bare
        // "host" entry means port 22 only. Match that exactly (don't also
        // accept the bare-hostname entry for a non-22 connection).
        let candidates: [String] = port == 22 ? [hostname] : ["[\(hostname)]:\(port)"]

        var trusted = false
        for rawLine in contents.split(separator: "\n", omittingEmptySubsequences: true) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty, !line.hasPrefix("#") else { continue }

            var fields = line.split(separator: " ", maxSplits: 4).map(String.init)
            guard fields.count >= 3 else { continue }

            // A leading @-marker (@revoked / @cert-authority) shifts every
            // field right by one. Pull it off so host/keytype/key line up.
            var marker: String?
            if fields[0].hasPrefix("@") {
                marker = fields.removeFirst()
                guard fields.count >= 3 else { continue }
            }

            // We don't support host certificates in the SFTP validator; a
            // @cert-authority line can't match a plain host key anyway.
            if marker == "@cert-authority" { continue }

            guard let candidateKey = try? NIOSSHPublicKey(openSSHPublicKey: "\(fields[1]) \(fields[2])"),
                  candidateKey == key else { continue }

            let hostMatches: Bool
            if fields[0].hasPrefix("|1|") {
                hostMatches = matchesHashedHost(fields[0], candidates: candidates)
            } else {
                hostMatches = fields[0].split(separator: ",").map(String.init).contains(where: candidates.contains)
            }
            guard hostMatches else { continue }

            // A key explicitly revoked for this host is refused outright,
            // overriding any trust line elsewhere in the file.
            if marker == "@revoked" { return false }
            trusted = true
        }
        return trusted
    }

    private static func matchesHashedHost(_ hashField: String, candidates: [String]) -> Bool {
        let parts = hashField.split(separator: "|", omittingEmptySubsequences: false)
        guard parts.count == 4,
              let salt = Data(base64Encoded: String(parts[2])),
              let expected = Data(base64Encoded: String(parts[3])) else { return false }

        for candidate in candidates {
            let mac = HMAC<Insecure.SHA1>.authenticationCode(
                for: Data(candidate.utf8),
                using: SymmetricKey(data: salt)
            )
            if Data(mac) == expected { return true }
        }
        return false
    }

    private static var knownHostsURL: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".ssh/known_hosts")
    }
}
