# Security

BHTerminal handles SSH/SFTP/VNC credentials, so security is taken seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Email
**benjamin dot biswas at gmail dot com** (or contact
[BiswasHost](https://www.biswashost.com)) with details and steps to reproduce.
You'll get a response as soon as possible.

## How BHTerminal protects your data (macOS build)

- **Passwords and key passphrases live only in the macOS Keychain** — never in
  the app's `sessions.json`, never in logs. The config file stores only a host's
  UUID, which the app uses to look the secret up.
- **The terminal runs your system `/usr/bin/ssh`** inside a pty, so host-key
  verification, known-hosts, your keys, and your SSH agent all work exactly as
  they do in Terminal.app. BHTerminal does not reimplement the SSH transport for
  interactive sessions.
- **The SFTP pane also runs over your real `ssh`** (the SFTP subsystem,
  reusing the terminal's authenticated connection), so it authenticates with
  your keys / agent / passphrase / 2FA / ProxyJump and gets the same
  `~/.ssh/known_hosts` host-key verification as the terminal — no separate
  credential path.
- **Argument-injection hardened.** Hostnames/usernames are allow-list validated
  (rejecting values like `-oProxyCommand=…`), and every `ssh` invocation ends
  option parsing with `--` before the destination, so a crafted host can't turn
  into a `ProxyCommand` and execute a command.
- **Imports are treated as untrusted.** Session/credential imports
  (MobaXterm / SSH config) re-validate every hostname and username before a host
  is saved, and clamp/strip imported names.
- **VNC clipboard sharing is off by default** (opt-in per host), since a hostile
  VNC server could otherwise read or poison the local clipboard.
- **Hardened Runtime** is enabled on release builds.

## The update check and the webview's network reach

The update check reads `https://api.github.com/repos/wpexpertinbd/BHTerminal/releases/latest`
and nothing else. That one origin is named in `connect-src`, and it is the entire
network reach of the webview: no remote content is ever loaded into it, and every
other origin is refused by the CSP.

The response is judged in Rust, not in the webview. The version comparison is
numeric (so 1.10 is newer than 1.9), the installer is matched to this platform and
architecture, and the download URL must be HTTPS on GitHub's own hosts —
`github.com`, `objects.githubusercontent.com` or a subdomain of either. A lookalike
such as `github.com.evil.example`, a URL carrying userinfo, or a bare IP address is
refused. That check runs again at the moment the URL is opened, because that is the
call that acts on it.

BHTerminal does not download or run an installer itself. It hands the URL to your
browser, so you can see what you are getting and where it came from before anything
is installed — which matters more than usual for an app that is self-signed.

## Backups

A backup holding saved passwords is always encrypted (AES-GCM under a PBKDF2 key,
210,000 iterations) and the passphrase is required, not optional — writing secrets
in the clear would undo the vault. Backup files are written owner-only (0600 on
macOS and Linux). A backup exported WITHOUT secrets is plain, readable JSON and
contains no credentials at all; that is the one to use for moving a host list
between computers.

Importing from MobaXterm or an `ssh_config` treats the file as untrusted input:
names are stripped of control characters and clamped in length, and every hostname
and username is re-checked against the ssh argument allow-list, so a crafted export
cannot smuggle an option such as `-oProxyCommand=…` into a saved host.

## Signing & notarization

BHTerminal is **self-signed and not notarized** (no paid Apple Developer
account). On first launch macOS flags it as an unverified developer; you approve
it once via **System Settings → Privacy & Security → Open Anyway**. See the main
[`macos/README.md`](macos/README.md). The signature is intact — verify with
`codesign --verify --deep --strict /Applications/BHTerminal.app`.

## Sensitive files, your side

MobaXterm's stored-passwords export is a **plaintext** file containing all your
passwords. After importing it into BHTerminal, **delete it** — the app's copy is
safe in the Keychain.
