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
        assert!(ssh::build_exit_master(&host(), &cm).unwrap().is_none());
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
