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
- **The SFTP pane verifies host keys against your `~/.ssh/known_hosts`** (both
  plain and HMAC-hashed entries) and **fails closed** for unknown hosts — it does
  not trust-on-first-use.
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
