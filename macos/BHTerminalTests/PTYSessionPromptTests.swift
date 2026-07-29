import XCTest
@testable import BHTerminal

/// Covers the bug where connecting hung forever on a key-passphrase prompt.
///
/// The terminal decides "login finished" by debouncing on an output lull — but
/// a prompt waiting for input looks exactly like a lull. It used to assume
/// quiet == ready, which wiped the Keychain passphrase before ssh had even
/// asked on any server slower than the debounce, leaving a prompt nothing would
/// answer (and telling the SFTP pane to connect to a session that wasn't up).
final class PTYSessionPromptTests: XCTestCase {

    func testRecognisesPromptsThatAreWaitingForInput() {
        let waiting = [
            "Enter passphrase for key '/Users/ben/.ssh/id_ed25519': ",
            "root@s2.bitsboxhost.com's password: ",
            "Password:",
            "Verification code: ",
            "(root@host) Password for root:",
            "Enter authentication code: ",
            "Duo two-factor login\n\nPasscode or option (1-3): ",
            "The authenticity of host 's2.example.com' can't be established.\n" +
            "ED25519 key fingerprint is SHA256:abc.\n" +
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            // Banner text before the prompt must not hide it.
            "Welcome to CWP\nLast login: Sun Jul 12\nEnter passphrase for key 'x': "
        ]
        for output in waiting {
            XCTAssertTrue(PTYSession.looksLikeAuthPrompt(output),
                          "should be treated as waiting for input: \(output.suffix(48))")
        }
    }

    func testDoesNotMistakeShellOutputForAPrompt() {
        let notWaiting = [
            "",
            "[root@s1 ~]# ",
            "root@s1:~$ ",
            "Last login: Sun Jul 12 06:50:29 2026 from 103.173.106.4\n",
            // Mentions a keyword but is finished output, not a prompt (newline).
            "Enter passphrase for key 'x': \n[root@s1 ~]# ",
            // Prompt-shaped but nothing to do with authentication.
            "Choose a region: ",
            "total 48\ndrwx------ 2 root root 4096 Jul 12 06:00 .ssh\n"
        ]
        for output in notWaiting {
            XCTAssertFalse(PTYSession.looksLikeAuthPrompt(output),
                           "should NOT be treated as an auth prompt: \(output.suffix(48))")
        }
    }
}
