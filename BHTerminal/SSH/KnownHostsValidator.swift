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

        let candidates: [String] = port == 22 ? [hostname] : [hostname, "[\(hostname)]:\(port)"]

        for rawLine in contents.split(separator: "\n", omittingEmptySubsequences: true) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty, !line.hasPrefix("#") else { continue }

            // host(s) keytype base64key [comment...] — maxSplits 3 keeps the
            // base64 key intact as its own field even when a comment follows.
            let fields = line.split(separator: " ", maxSplits: 3).map(String.init)
            guard fields.count >= 3 else { continue }

            guard let candidateKey = try? NIOSSHPublicKey(openSSHPublicKey: "\(fields[1]) \(fields[2])"),
                  candidateKey == key else { continue }

            let hostField = fields[0]
            if hostField.hasPrefix("|1|") {
                if matchesHashedHost(hostField, candidates: candidates) { return true }
            } else if hostField.split(separator: ",").map(String.init).contains(where: candidates.contains) {
                return true
            }
        }
        return false
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
