//! Telling "ssh is waiting for a human" apart from "the shell is ready".
//!
//! This exists because of a bug that cost a release. The terminal decides a
//! login has finished by waiting for output to go quiet — but a prompt
//! waiting for input looks exactly like quiet. The old build assumed quiet
//! meant ready, so on any server slower than the debounce it discarded the
//! saved passphrase BEFORE ssh asked for it: the session then sat forever on
//! a prompt nothing would answer, and the file pane was told to connect to a
//! session that wasn't up. Diagnosed on a live machine with ssh ESTABLISHED
//! and blocked in pselect.
//!
//! So the rule is: if the tail of the output looks like a prompt, the login
//! is NOT finished, whatever the silence suggests.

/// Fragments that mean ssh wants something typed.
const AUTH_NEEDLES: &[&str] = &[
    "passphrase for",
    "password:",
    "password for",
    "verification code",
    "authentication code",
    "one-time password",
    "passcode",
    "token:",
    "otp:",
    "(yes/no",
    "authenticity of host",
];

/// Removes terminal control sequences, leaving the text a person would read.
///
/// ⚠️ Raw ssh output is clean on macOS and Linux, but on Windows the pty is a
/// ConPTY, which RE-DRAWS the screen: the prompt arrives wrapped in cursor
/// moves and `ESC[?25h`-style sequences, often with one trailing it. Matching
/// the prompt against that raw stream means "ends with a colon" is never true,
/// so a saved password is never offered. Strip first, match second.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters/intermediates, then one final byte 0x40–0x7E.
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC (window title and friends): ends at BEL or ESC \.
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // DCS / SOS / PM / APC: strings that end at ESC \.
                Some('P' | 'X' | '^' | '_') => {
                    while let Some(n) = chars.next() {
                        if n == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Character-set selection takes one more byte.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                // Any other two-byte escape (ESC 7, ESC =, ESC c …).
                _ => {}
            },
            // Keep line structure; drop bells, backspaces and the rest.
            '\n' | '\r' | '\t' => out.push(c),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {}
            c => out.push(c),
        }
    }
    out
}

/// The last line a person would see, after control sequences and carriage
/// returns are accounted for (`\rprompt` is how ssh redraws a line).
fn last_visible_line(output: &str) -> String {
    let clean = strip_ansi(output);
    let line = clean.rsplit('\n').next().unwrap_or("");
    line.rsplit('\r')
        .find(|seg| !seg.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Whether the last line of `output` is a prompt waiting for input.
///
/// Prompts do not end in a newline, so the tail ends with ":" / "?" / a
/// "(yes/no…" fragment. Both halves matter: requiring only the keyword would
/// match a finished line that merely mentions a password, and requiring only
/// the punctuation would match any shell prompt.
pub fn looks_like_auth_prompt(output: &str) -> bool {
    let last_line = last_visible_line(output).to_lowercase();
    if last_line.is_empty() {
        return false;
    }
    let prompt_shaped =
        last_line.ends_with(':') || last_line.ends_with('?') || last_line.contains("(yes/no");
    if !prompt_shaped {
        return false;
    }
    AUTH_NEEDLES.iter().any(|n| last_line.contains(n))
}

/// Which secret, if any, answers the prompt currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Password,
    Passphrase,
    /// A host-key confirmation or a 2FA code — only the user can answer.
    Human,
}

/// Classifies the prompt so the caller sends the right secret, and never
/// sends a stored password in answer to a host-key question.
pub fn what_is_wanted(output: &str) -> Option<Wanted> {
    if !looks_like_auth_prompt(output) {
        return None;
    }
    let tail = last_visible_line(output).to_lowercase();
    if tail.contains("passphrase for") {
        Some(Wanted::Passphrase)
    } else if tail.contains("(yes/no") || tail.contains("authenticity of host") {
        Some(Wanted::Human)
    } else if tail.contains("password") {
        Some(Wanted::Password)
    } else {
        // A 2FA/verification code: we have nothing stored that answers it.
        Some(Wanted::Human)
    }
}
