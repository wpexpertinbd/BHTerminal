//! Making a key file private: reading icacls/whoami output and deciding.

use bhterm_core::keyperms::*;

#[test]
fn whoami_gives_the_name_and_sid() {
    let csv = "\"desktop-1\\jhinuk\",\"S-1-5-21-714424246-1328522028-26754142-1001\"\r\n";
    assert_eq!(
        parse_whoami_user(csv),
        Some(("desktop-1\\jhinuk".to_string(), "S-1-5-21-714424246-1328522028-26754142-1001".to_string()))
    );
    assert_eq!(parse_whoami_user("garbage"), None);
    assert_eq!(parse_whoami_user("\"a\",\"not-a-sid\""), None);
}

#[test]
fn icacls_lists_every_principal_on_a_key_with_spaces_in_names() {
    let path = "C:\\Users\\Jhinuk\\Downloads\\Office-PC";
    let out = format!(
        "{path} DESKTOP-1\\CodexSandboxUsers:(I)(RX)\r\n\
         {pad}NT AUTHORITY\\Authenticated Users:(I)(M)\r\n\
         {pad}NT AUTHORITY\\SYSTEM:(I)(F)\r\n\
         {pad}DESKTOP-1\\jhinuk:(I)(F)\r\n\
         \r\n\
         Successfully processed 1 files; Failed processing 0 files\r\n",
        pad = " ".repeat(path.len() + 1)
    );
    let got = parse_icacls(&out, path);
    assert_eq!(
        got,
        vec![
            "DESKTOP-1\\CodexSandboxUsers",
            "NT AUTHORITY\\Authenticated Users",
            "NT AUTHORITY\\SYSTEM",
            "DESKTOP-1\\jhinuk",
        ]
    );
    assert_eq!(
        principals_to_remove(&got, "desktop-1\\JHINUK"),
        vec!["DESKTOP-1\\CodexSandboxUsers", "NT AUTHORITY\\Authenticated Users", "NT AUTHORITY\\SYSTEM"],
        "everyone but the user goes; names compare without case"
    );
}

#[test]
fn a_private_key_has_only_the_user_left() {
    let path = "F:\\Office-PC";
    let out = format!("{path} DESKTOP-1\\jhinuk:(F)\r\n\r\nSuccessfully processed 1 files\r\n");
    let got = parse_icacls(&out, path);
    assert!(principals_to_remove(&got, "DESKTOP-1\\jhinuk").is_empty());
}

#[test]
fn the_commands_name_the_user_by_sid_and_pass_the_path_as_one_argument() {
    let p = "C:\\Users\\A B\\.ssh\\id key";
    // Every call carries /L, so a file swapped for a link is never followed.
    assert_eq!(build_icacls_restrict(p, "S-1-5-21-1"), vec![p, "/inheritance:r", "/grant:r", "*S-1-5-21-1:F", "/L"]);
    assert_eq!(build_icacls_remove(p, "NT AUTHORITY\\Authenticated Users"), vec![p, "/remove:g", "NT AUTHORITY\\Authenticated Users", "/L"]);
    assert_eq!(build_icacls_owner(p, "S-1-5-21-1"), vec![p, "/setowner", "*S-1-5-21-1", "/L"]);
    assert_eq!(build_icacls_list(p), vec![p, "/L"]);
}

#[test]
fn unix_modes() {
    assert!(unix_mode_is_private(0o600));
    assert!(unix_mode_is_private(0o400));
    assert!(!unix_mode_is_private(0o644));
    assert!(!unix_mode_is_private(0o640));
}

#[test]
fn the_readback_proves_privacy_only_when_it_worked_and_shows_just_the_user() {
    let me = "DESKTOP-1\\jhinuk";
    let only_me = vec![me.to_string()];
    assert!(acl_is_private(&only_me, me).is_ok());
    assert!(acl_is_private(&[], me).is_err(), "a failed or empty read is not proof");
    assert!(acl_is_private(&["NT AUTHORITY\\SYSTEM".to_string()], me).is_err(), "the user must be on it");
    assert!(acl_is_private(&[me.to_string(), "Everyone".to_string()], me).is_err());
}

#[test]
fn a_garbled_account_name_is_never_trusted() {
    // A non-ASCII name printed in a non-UTF-8 console code page.
    let garbled = "DESKTOP-1\\Jos\u{FFFD}";
    assert!(!name_is_trustworthy(garbled));
    assert!(acl_is_private(&[garbled.to_string()], garbled).is_err());
    assert!(name_is_trustworthy("DESKTOP-1\\jhinuk"));
}
