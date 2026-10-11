//! cwd-follow reads the window title, so nothing is ever injected into the
//! user's shell. These pin the real distro formats.

use bhterm_core::title;

#[test]
fn parses_real_distro_title_formats() {
    let cases = [
        ("root@s1:~", "~"),                       // CentOS/RHEL/Alma /etc/bashrc
        ("root@s1:/home", "/home"),
        ("root@s1:~/public_html", "~/public_html"),
        ("benjamin@box: ~/src/app", "~/src/app"), // Debian/Ubuntu ~/.bashrc
        ("benjamin@box: /var/log", "/var/log"),
        ("~/dir", "~/dir"),
        ("/usr/local/bin", "/usr/local/bin"),
        ("  root@s1:/etc/nginx  ", "/etc/nginx"),
    ];
    for (title, want) in cases {
        assert_eq!(title::parse_title(title).as_deref(), Some(want), "title: {title}");
    }
}

#[test]
fn ignores_titles_that_are_not_paths() {
    for t in ["", "   ", "vim foo.txt", "htop", "root@s1", "ssh root@s1", "man: ssh",
              "root@s1:relative/dir", "Choose a region: "] {
        assert_eq!(title::parse_title(t), None, "title: {t}");
    }
}

#[test]
fn ambiguous_and_hostile_titles_are_ignored() {
    // A path containing a colon can't be split safely — better to stay put
    // than navigate somewhere wrong.
    assert_eq!(title::parse_title("root@s1:/data/a:b"), None);
    // A hostile title must not smuggle escape sequences into a navigation.
    assert_eq!(title::parse_title("root@s1:/tmp/\u{1b}[2Jevil"), None);
    assert_eq!(title::parse_title("root@s1:/tmp/\u{7}evil"), None);
}

#[test]
fn osc7_is_still_honoured_when_the_remote_has_shell_integration() {
    assert_eq!(title::parse_osc7("file://host/var/www").as_deref(), Some("/var/www"));
    assert_eq!(title::parse_osc7("/plain/path").as_deref(), Some("/plain/path"));
    assert_eq!(title::parse_osc7(""), None);
    assert_eq!(title::parse_osc7("not-a-path"), None);
}

#[test]
fn tilde_expands_against_the_remote_home_only_once_known() {
    assert_eq!(title::expand_tilde("~", Some("/root")).as_deref(), Some("/root"));
    assert_eq!(title::expand_tilde("~/x", Some("/root")).as_deref(), Some("/root/x"));
    assert_eq!(title::expand_tilde("~/x", Some("/root/")).as_deref(), Some("/root/x"));
    assert_eq!(title::expand_tilde("/abs", None).as_deref(), Some("/abs"));
    // Unknown home: stay put rather than guess.
    assert_eq!(title::expand_tilde("~/x", None), None);
}
