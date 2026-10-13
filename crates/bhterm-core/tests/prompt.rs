//! Covers the bug where connecting hung forever: a saved passphrase was
//! discarded before ssh asked for it, because silence was mistaken for a
//! finished login.

use bhterm_core::prompt::{looks_like_auth_prompt, what_is_wanted, Wanted};

#[test]
fn recognises_prompts_that_are_waiting_for_input() {
    for s in [
        "Enter passphrase for key '/Users/ben/.ssh/id_ed25519': ",
        "root@s2.example.com's password: ",
        "Password:",
        "Verification code: ",
        "(root@host) Password for root:",
        "Duo two-factor login\n\nPasscode or option (1-3): ",
        "The authenticity of host 's2.example.com' can't be established.\n\
         ED25519 key fingerprint is SHA256:abc.\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
        "Welcome to CWP\nLast login: Sun\nEnter passphrase for key 'x': ",
    ] {
        assert!(looks_like_auth_prompt(s), "should be waiting: {:?}", &s[s.len().saturating_sub(40)..]);
    }
}

#[test]
fn ordinary_shell_output_is_not_a_prompt() {
    for s in [
        "",
        "   ",
        "[root@s1 ~]# ",
        "root@s1:~$ ",
        "Last login: Sun Jul 12 06:50:29 2026 from 10.0.0.1\n",
        // Mentions the word but is finished output, not a prompt.
        "Enter passphrase for key 'x': \n[root@s1 ~]# ",
        // Prompt-shaped but nothing to do with authentication.
        "Choose a region: ",
        "total 48\ndrwx------ 2 root root 4096 .ssh\n",
    ] {
        assert!(!looks_like_auth_prompt(s), "should NOT be waiting: {s:?}");
    }
}

#[test]
fn sends_the_right_secret_and_never_guesses_at_a_host_key() {
    assert_eq!(what_is_wanted("Enter passphrase for key 'x': "), Some(Wanted::Passphrase));
    assert_eq!(what_is_wanted("root@host's password: "), Some(Wanted::Password));
    // A stored password must never be typed at a host-key question, nor at a
    // 2FA code prompt — only the user can answer those.
    assert_eq!(
        what_is_wanted("Are you sure you want to continue connecting (yes/no)? "),
        Some(Wanted::Human)
    );
    assert_eq!(what_is_wanted("Verification code: "), Some(Wanted::Human));
    assert_eq!(what_is_wanted("[root@s1 ~]# "), None);
}

/// Windows runs the terminal through ConPTY, which re-draws the screen: the
/// prompt arrives with a window-title sequence in front and cursor/visibility
/// codes behind. Matched raw, "ends with a colon" was never true there, so a
/// saved password was never offered.
#[test]
fn a_prompt_wrapped_in_conpty_control_codes_is_still_a_prompt() {
    for s in [
        "\x1b[?25l\x1b]0;C:\\Windows\\System32\\OpenSSH\\ssh.exe\x07root@host's password: \x1b[?25h",
        "\x1b[25;1Hroot@host's password: \x1b[25;28H",
        "\x1b[2J\x1b[H\x1b[mbenjamin@10.0.0.5's password: \x1b[?25h\x1b[25;31H",
        "\x1b[1mEnter passphrase for key 'C:\\Users\\b\\.ssh\\id_ed25519':\x1b[0m ",
        "\rroot@host's password: ",
    ] {
        assert!(looks_like_auth_prompt(s), "should be waiting: {s:?}");
    }
    assert_eq!(
        what_is_wanted("\x1b[?25lroot@host's password: \x1b[?25h"),
        Some(Wanted::Password)
    );
    assert_eq!(
        what_is_wanted("\x1b]0;ssh\x07\x1b[1mEnter passphrase for key 'x':\x1b[0m "),
        Some(Wanted::Passphrase)
    );
}

#[test]
fn control_codes_do_not_turn_ordinary_output_into_a_prompt() {
    for s in [
        "\x1b[32m[root@s1 ~]# \x1b[0m",
        "\x1b]0;root@s1:~\x07[root@s1 ~]# ",
        // A prompt that was answered: the newline after it ends the wait.
        "root@host's password: \r\n\x1b[?25h",
    ] {
        assert!(!looks_like_auth_prompt(s), "should NOT be waiting: {s:?}");
    }
}

#[test]
fn strip_ansi_keeps_the_words_and_drops_the_codes() {
    use bhterm_core::prompt::strip_ansi;
    assert_eq!(strip_ansi("\x1b[1;31mred\x1b[0m text"), "red text");
    assert_eq!(strip_ansi("a\x1b]0;title\x07b"), "ab");
    assert_eq!(strip_ansi("a\x1b]0;title\x1b\\b"), "ab");
    assert_eq!(strip_ansi("a\x1b(Bb"), "ab");
    // An escape cut off at the end of a chunk must not leak into the text.
    assert_eq!(strip_ansi("password:\x1b[?2"), "password:");
    assert_eq!(strip_ansi("password:\x1b"), "password:");
    // Line structure survives; bells and backspaces do not.
    assert_eq!(strip_ansi("a\r\nb\x07\x08"), "a\r\nb");
    // Non-ASCII text is untouched.
    assert_eq!(strip_ansi("\x1b[mপাসওয়ার্ড:"), "পাসওয়ার্ড:");
}
