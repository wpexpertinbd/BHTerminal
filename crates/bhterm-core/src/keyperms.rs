//! Making a private key file private again, so ssh will use it.
//!
//! ssh refuses a key that other accounts can read ("UNPROTECTED PRIVATE KEY
//! FILE … This private key will be ignored"). On macOS/Linux that is the
//! file mode; on Windows it is the file's ACL — keys copied to a second drive
//! carry "Authenticated Users", and folders in the profile can carry extra
//! groups (a sandbox's, for one). The shell runs the commands; this module
//! only reads their output and decides, so it can be tested anywhere.

/// The account running the app, from `whoami /user /fo csv /nh`:
/// `"desktop-1\jhinuk","S-1-5-21-…-1001"` → (name, SID).
pub fn parse_whoami_user(csv: &str) -> Option<(String, String)> {
    let line = csv.lines().map(str::trim).find(|l| !l.is_empty())?;
    let fields: Vec<&str> = line.split("\",\"").collect();
    if fields.len() != 2 {
        return None;
    }
    let name = fields[0].trim_start_matches('"').to_string();
    let sid = fields[1].trim_end_matches('"').to_string();
    if name.is_empty() || !sid.starts_with("S-1-") {
        return None;
    }
    Some((name, sid))
}

/// Who has an entry on the file, from `icacls <path>`. The first line starts
/// with the path itself; the list ends at the first blank line (after it
/// comes a localised "Successfully processed …" summary).
///
/// ```text
/// F:\Office-PC NT AUTHORITY\Authenticated Users:(I)(M)
///              NT AUTHORITY\SYSTEM:(I)(F)
///
/// Successfully processed 1 files; Failed processing 0 files
/// ```
pub fn parse_icacls(output: &str, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, raw) in output.lines().enumerate() {
        let line = if i == 0 {
            match raw.strip_prefix(path) {
                Some(rest) => rest,
                None => raw,
            }
        } else {
            raw
        };
        let line = line.trim();
        if line.is_empty() {
            if i == 0 {
                continue;
            }
            break;
        }
        if let Some(at) = line.find(":(") {
            out.push(line[..at].trim().to_string());
        }
    }
    out
}

/// The entries to take away so that only `me` keeps access. Compared without
/// case, as Windows does.
pub fn principals_to_remove(principals: &[String], me: &str) -> Vec<String> {
    let mut out: Vec<String> = principals
        .iter()
        .filter(|p| !p.eq_ignore_ascii_case(me))
        .cloned()
        .collect();
    out.dedup();
    out
}

/// argv for `icacls` that drops inherited entries and gives the user (by SID,
/// so a localised or renamed account still matches) full control.
///
/// Every icacls call carries `/L`: if the file were swapped for a symbolic
/// link in the meantime, icacls acts on the link itself instead of following
/// it to some other file.
pub fn build_icacls_restrict(path: &str, sid: &str) -> Vec<String> {
    vec![
        path.to_string(),
        "/inheritance:r".into(),
        "/grant:r".into(),
        format!("*{sid}:F"),
        "/L".into(),
    ]
}

/// argv for `icacls` that removes one remaining entry by name.
pub fn build_icacls_remove(path: &str, principal: &str) -> Vec<String> {
    vec![path.to_string(), "/remove:g".into(), principal.to_string(), "/L".into()]
}

/// argv for `icacls` that makes the user the owner (ssh also checks that).
pub fn build_icacls_owner(path: &str, sid: &str) -> Vec<String> {
    vec![path.to_string(), "/setowner".into(), format!("*{sid}"), "/L".into()]
}

/// argv for `icacls` that lists the file's entries.
pub fn build_icacls_list(path: &str) -> Vec<String> {
    vec![path.to_string(), "/L".into()]
}

/// Windows console tools print names in the console code page, which is not
/// UTF-8; a non-ASCII name decodes with U+FFFD replacement characters, and two
/// different accounts can then look identical. Such a name cannot be used to
/// decide who keeps access.
pub fn name_is_trustworthy(name: &str) -> bool {
    !name.is_empty() && !name.contains('\u{FFFD}')
}

/// The ACL read back after the change proves the key is private only if the
/// read worked, it lists the user, and it lists nobody else.
pub fn acl_is_private(principals: &[String], me: &str) -> Result<(), String> {
    if principals.is_empty() {
        return Err("couldn't read the file's permissions back".into());
    }
    if let Some(bad) = principals.iter().find(|p| !name_is_trustworthy(p)) {
        return Err(format!("couldn't read an account name on it reliably ({bad})"));
    }
    if !principals.iter().any(|p| p.eq_ignore_ascii_case(me)) {
        return Err("your own account isn't on it".into());
    }
    let others = principals_to_remove(principals, me);
    if !others.is_empty() {
        return Err(format!("it can still be read by: {}", others.join(", ")));
    }
    Ok(())
}

/// A Unix file mode is private enough for ssh when neither group nor others
/// can do anything with it.
pub fn unix_mode_is_private(mode: u32) -> bool {
    mode & 0o077 == 0
}
