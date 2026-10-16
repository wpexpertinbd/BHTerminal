//! Handing a saved password to an `ssh` that has no terminal — Windows only
//! in practice, where OpenSSH cannot share the terminal's connection and the
//! file pane must log in on its own.
//!
//! ssh asks for a password by running the program named in `SSH_ASKPASS` and
//! reading its stdout. That program is THIS executable, started in a
//! one-shot mode that is selected only by the presence of
//! `BHTERMINAL_ASKPASS_SECRET` in its environment — which the file pane sets
//! on the single ssh child it spawns, and nothing else ever does. The secret
//! therefore never appears on a command line (visible to `tasklist`/`ps`);
//! it lives in the child's environment, readable by the same user only.
//!
//! The helper answers ONE question and nothing else: "what is the password
//! for <user@host>" for the exact `user@host` it was started for
//! (`BHTERMINAL_ASKPASS_FOR`). ⚠️ It must be that narrow. The ssh child hands
//! its environment to anything IT starts — a `-J` jump host, a `ProxyCommand`
//! — and a jump host's own prompt ("jumpuser@jump's password:") is also a
//! password prompt. Answering on the word "password" alone would have sent the
//! TARGET's password to a different server. A passphrase, a host-key question,
//! a code or a prompt for another account is refused (exit 1).

use std::io::Write;

pub const SECRET_VAR: &str = "BHTERMINAL_ASKPASS_SECRET";
/// The `user@host` the secret belongs to; a prompt for anything else is refused.
pub const TARGET_VAR: &str = "BHTERMINAL_ASKPASS_FOR";

/// Words that make a prompt something other than "your current password".
const NOT_THE_PASSWORD: [&str; 10] = [
    "passphrase", "(yes/no", "new password", "retype", "re-enter", "old password",
    "current password", "one-time", "token", "passcode",
];

/// Whether `prompt` is ssh asking for the account password of `target`
/// (`user@host`). Only the two shapes OpenSSH itself prints are accepted, and
/// they must START with the target's name:
///   `user@host's password: `       (password authentication)
///   `(user@host) Password: `       (keyboard-interactive / PAM)
pub fn wants_password(prompt: &str, target: &str) -> bool {
    let target = target.trim().to_lowercase();
    if target.is_empty() {
        return false;
    }
    let p = prompt.trim_start().to_lowercase();
    let named = p.starts_with(&format!("{target}'s password"))
        || p.starts_with(&format!("({target}) password"));
    named && !NOT_THE_PASSWORD.iter().any(|w| p.contains(w))
}

/// The environment that makes an ssh child fetch `secret` for `target`
/// (`user@host`) through this executable. `SSH_ASKPASS_REQUIRE=force`
/// (OpenSSH 8.4+) uses the helper even without a display; `DISPLAY` is set
/// because older builds only consult askpass when it is non-empty.
pub fn env_for(secret: &str, target: &str) -> Result<Vec<(String, String)>, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("can't locate BHTerminal to use as the password helper: {e}"))?;
    Ok(vec![
        ("SSH_ASKPASS".into(), exe.to_string_lossy().into_owned()),
        ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
        ("DISPLAY".into(), "bhterminal:0".into()),
        (TARGET_VAR.into(), target.to_string()),
        (SECRET_VAR.into(), secret.to_string()),
    ])
}

/// Called first thing at startup. If this process was started by ssh as the
/// askpass helper, answer and exit; otherwise return and let the app start.
pub fn run_if_helper() {
    let Some(secret) = std::env::var_os(SECRET_VAR) else { return };
    // The prompt text comes from a SERVER. `args_os`, so a prompt that is not
    // valid UTF-8 cannot panic the helper.
    let prompt = std::env::args_os()
        .nth(1)
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = std::env::var(TARGET_VAR).unwrap_or_default();
    if wants_password(&prompt, &target) {
        let mut out = std::io::stdout();
        let ok = out
            .write_all(secret.to_string_lossy().as_bytes())
            .and_then(|_| out.write_all(b"\n"))
            .and_then(|_| out.flush())
            .is_ok();
        // A failed write must not look like success: ssh would take an empty
        // answer as a (failed) password attempt.
        std::process::exit(if ok { 0 } else { 1 });
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "root@203.0.113.9";

    #[test]
    fn only_the_targets_own_password_prompt_is_answered() {
        for yes in [
            "root@203.0.113.9's password: ",
            "ROOT@203.0.113.9's password:",
            "(root@203.0.113.9) Password: ",
            "(root@203.0.113.9) Password for root: ",
        ] {
            assert!(wants_password(yes, T), "should answer: {yes:?}");
        }
        for no in [
            // A jump host's prompt is ALSO a password prompt — the reason for the binding.
            "jumpuser@198.51.100.1's password: ",
            "(jumpuser@198.51.100.1) Password: ",
            // Another account on the same machine, or a lookalike prefix.
            "admin@203.0.113.9's password: ",
            "xroot@203.0.113.9's password: ",
            // Things that are not "your current password".
            "(root@203.0.113.9) New password: ",
            "(root@203.0.113.9) Retype new password: ",
            "(root@203.0.113.9) One-time password: ",
            "root@203.0.113.9's password (token): ",
            "Enter passphrase for key '/home/b/.ssh/id_ed25519': ",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            "Verification code: ",
            "Password: ",
            "",
        ] {
            assert!(!wants_password(no, T), "must NOT answer: {no:?}");
        }
        // With no target to bind to, it answers nothing at all.
        assert!(!wants_password("root@203.0.113.9's password: ", ""));
        assert!(!wants_password("root@203.0.113.9's password: ", "   "));
    }

    #[test]
    fn the_hand_over_environment_names_this_executable_the_target_and_the_secret() {
        let env = env_for("s3cret value", T).unwrap();
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("SSH_ASKPASS_REQUIRE"), Some("force"));
        assert_eq!(get(SECRET_VAR), Some("s3cret value"));
        assert_eq!(get(TARGET_VAR), Some(T));
        assert!(!get("DISPLAY").unwrap().is_empty());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(get("SSH_ASKPASS"), Some(exe.to_string_lossy().as_ref()));
        // The secret travels ONLY in the environment, never in an argument.
        assert!(!env.iter().any(|(k, v)| k.starts_with("SSH_") && v.contains("s3cret")));
    }
}
