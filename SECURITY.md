# Security

BHTerminal handles SSH/SFTP/VNC credentials, so security is taken seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Email
**benjamin dot biswas at gmail dot com** (or contact
[BiswasHost](https://www.biswashost.com)) with details and steps to reproduce.
You'll get a response as soon as possible.

## How BHTerminal protects your data

- **Passwords and key passphrases live only in an encrypted vault file**
  (`secrets.json`, AES-GCM, owner-only) — never in the app's `sessions.json`,
  never in logs. The config file stores only a host's UUID, which the app uses
  to look the secret up. (Not the OS keychain: a self-signed app looks like a
  new application to the macOS Keychain after every update, so it re-prompted
  constantly.)
- **A saved password belongs to the server it was saved for.** Changing a
  host's address, port, user or jump host forgets its saved password unless a
  new one is typed — so a host cannot be quietly re-pointed at someone else's
  server to collect the real password.
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

## Master password

Optional, and asked about once. Without one, the vault's key sits beside it in
an owner-only file and the app unlocks by itself: that protects a **copy** of
the data folder (a backup, a synced folder), not someone using your account.

With one, the key is sealed under the master password (PBKDF2-HMAC-SHA256,
600,000 iterations, AES-GCM). "Remember on this computer" (off by default)
keeps the plain key on that computer, which is the same protection as having no
master password there; left off, nothing on disk opens the vault until the
password is typed. Setting, changing or un-remembering it moves the vault to a
**new** key, so an old copy of the plain key opens nothing saved since.
Changing it, removing it, and exporting saved passwords all need it.

What it does **not** do, stated plainly:

- It is asked when BHTerminal **starts**. Closing the window keeps the app
  running in the menu bar, unlocked; quit it to lock.
- While unlocked, the app is your account: it has a local shell. Lock your
  computer when you walk away.
- The host list (`sessions.json`) is not encrypted — names and addresses are
  visible without the master password, and "Forgot it?" opens the app with the
  host list and an empty password store. The locked passwords are moved aside,
  not destroyed.
- A forgotten master password **cannot be recovered**, by you or by BHTerminal.
- Anyone at an unlocked app with no master password set can set one — and lock
  you out of the saved passwords. Set one yourself if that matters to you.

## Backups

A backup holding saved passwords is always encrypted (AES-GCM under a PBKDF2 key,
600,000 iterations) and a password is required — the master password by default
when one is set, or a passphrase of your own. It is not optional: writing
secrets in the clear would undo the vault. Backup files are written owner-only (0600 on
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
safe in its encrypted vault.
