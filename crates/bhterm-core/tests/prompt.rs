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
