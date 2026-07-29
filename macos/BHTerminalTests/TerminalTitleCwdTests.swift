import XCTest
@testable import BHTerminal

/// cwd-follow used to work by typing a PROMPT_COMMAND hook into the user's
/// session, which the remote shell echoed as a wall of shell code. The cwd is
/// now read from the window title that distro defaults already emit, so nothing
/// is ever injected. These cover the parsing.
final class TerminalTitleCwdTests: XCTestCase {

    func testParsesRealDistroTitleFormats() {
        let cases: [(String, String)] = [
            // CentOS / RHEL / AlmaLinux /etc/bashrc: "\u@\h:${PWD/#$HOME/~}"
            ("root@s1:~", "~"),
            ("root@s1:/home", "/home"),
            ("root@s1:~/public_html", "~/public_html"),
            // Debian / Ubuntu ~/.bashrc: "\u@\h: \w"
            ("benjamin@box: ~/src/app", "~/src/app"),
            ("benjamin@box: /var/log", "/var/log"),
            // Bare paths (some zsh/shell integrations)
            ("~/dir", "~/dir"),
            ("/usr/local/bin", "/usr/local/bin"),
            ("~", "~"),
            // Surrounding whitespace is ignored.
            ("  root@s1:/etc/nginx  ", "/etc/nginx")
        ]
        for (title, expected) in cases {
            XCTAssertEqual(TerminalTitleCwd.parsePath(fromTitle: title), expected,
                           "title: \(title)")
        }
    }

    func testIgnoresTitlesThatAreNotPaths() {
        let ignored = [
            "",
            "   ",
            "vim foo.txt",
            "htop",
            "root@s1",                 // no path part at all
            "Terminal",
            "ssh root@s1",
            "man: ssh",                // colon but not a path
            "root@s1:relative/dir"     // not absolute and not ~
        ]
        for title in ignored {
            XCTAssertNil(TerminalTitleCwd.parsePath(fromTitle: title), "title: \(title)")
        }
    }

    /// A path containing a colon can't be split unambiguously, so it's ignored
    /// rather than risking navigating somewhere wrong.
    func testAmbiguousPathWithColonIsIgnored() {
        XCTAssertNil(TerminalTitleCwd.parsePath(fromTitle: "root@s1:/data/a:b"))
    }

    /// A hostile title must not smuggle control characters into a path.
    func testRejectsControlCharacters() {
        XCTAssertNil(TerminalTitleCwd.parsePath(fromTitle: "root@s1:/tmp/\u{07}evil"))
        XCTAssertNil(TerminalTitleCwd.parsePath(fromTitle: "root@s1:/tmp/\u{1B}[2Jevil"))
    }
}
