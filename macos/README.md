# BHTerminal — macOS

> The **macOS build** of BHTerminal. For the cross-platform overview and the
> Windows/Linux ports, see the [repository root](../README.md).

A native macOS SSH / SFTP / VNC client — a MobaXterm-style workspace with a
session tree, a real terminal, a file browser, and remote-desktop tabs, all in
one window.

Built by [BiswasHost](https://www.biswashost.com). Requires macOS 14+
(Apple Silicon or Intel).

## Features

- **Sessions sidebar** — organize hosts in folders, quick-connect filter,
  double-click to connect.
- **Real SSH terminal** — spawns your system `ssh`, so `~/.ssh/config`, the SSH
  agent, `ProxyJump`, keyboard-interactive/2FA, and host-key prompts all just
  work. Tabs + split panes, broadcast-input to every pane, snippets, and
  optional session logging.
- **SFTP file browser** — drag-and-drop upload/download, rename/delete/mkdir,
  and a breadcrumb path that **follows your terminal's directory** as you `cd`.
- **VNC** — connect to remote desktops in the same tabbed window (not a separate
  app), password auth, opt-in clipboard sharing.
- **SSH tunnels** — local / remote / dynamic (SOCKS) port forwards, managed per
  host, that keep running in the background.
- **Menu-bar mode** — close the window and BHTerminal keeps running in the menu
  bar (tunnels stay up); reopen or quit from the top-bar icon.
- **Launch at login** — optional, in Preferences.
- **Secure by design** — passwords/passphrases live only in the macOS Keychain
  (never in the config file), the SFTP connection verifies host keys against
  `~/.ssh/known_hosts`, and the app ships with Hardened Runtime.

## Requirements

- macOS 14 (Sonoma) or later — Apple Silicon or Intel.

## Install

Download the latest **`BHTerminal-x.y.z.dmg`** or **`.pkg`** from the
[Releases](https://github.com/wpexpertinbd/BHTerminal/releases/latest) page.

- **DMG:** open it, drag **BHTerminal** onto the **Applications** folder.
- **PKG:** open it and follow the installer (installs to `/Applications`).

## First launch — allowing an unsigned app

BHTerminal is signed but **not notarized by Apple** (notarization requires a paid
Apple Developer account). So the first time you open it, macOS will say it
*"could not verify BHTerminal is free of malware."* This is expected — you just
approve it once:

**On macOS 15 (Sequoia) / 26 (Tahoe):**
1. Double-click **BHTerminal** — macOS blocks it and shows the warning. Click
   **Done**.
2. Open **System Settings → Privacy & Security**, scroll down to the
   *"BHTerminal was blocked…"* message, and click **Open Anyway**.
3. Confirm with Touch ID / your password. BHTerminal opens and won't ask again.

*(On the `.pkg`, if the installer refuses to open, right-click it in Finder →
**Open** → **Open**.)*

You can verify the signature yourself in Terminal:

```sh
codesign -dvv /Applications/BHTerminal.app     # Authority=BHTerminal Dev
codesign --verify --deep --strict /Applications/BHTerminal.app   # (no output = valid)
```

## Saved-password prompt

The first time BHTerminal reads a saved host password, macOS asks for your
**login keychain password** (your Mac account password). Enter it and click
**Always Allow** — you'll only see this once. (BHTerminal is signed with a
stable certificate, so the grant persists across app updates.)

## Migrating from MobaXterm

**Import Sessions…** lives in the sidebar's **`+`** menu (and **File → Import
Sessions…**, ⇧⌘I). Two MobaXterm exports are supported:

### Option A — bring your passwords across (recommended)

MobaXterm's password export includes host, port, user **and** the saved
password, so you get a complete migration:

1. In MobaXterm: **Settings → Configuration → General → "MobaXterm passwords
   management" → Export to file**. It asks for your **master password** and
   saves a `.txt`.
2. In BHTerminal: sidebar **`+` → Import Sessions…** → pick that `.txt`.
   Every host is created with its password stored in your **macOS Keychain**.
3. ⚠️ **Delete the `.txt` afterward** — it holds all your passwords in
   cleartext. BHTerminal's copy is safe in the Keychain.

*Note:* this export contains no session names or folders (MobaXterm keeps
those separately), so hosts arrive named `user@host` in one folder. Use
Option B as well if you want your names and folder tree.

### Option B — bring your names + folder structure (no passwords)

1. In MobaXterm: right-click **User sessions → Export sessions (.mxtsessions)**.
2. Import that file — session names and nested folders come across; re-enter
   passwords per host (they go into the Keychain).

### Not supported: the encrypted `.mobaconf` backup

A master-password-protected **full configuration** (`.mobaconf`) is encrypted
with an undocumented scheme and can't be read directly — export via Option A or
B instead. (BHTerminal detects an encrypted `.mobaconf` and tells you this.)

### From an SSH config

An OpenSSH `~/.ssh/config` imports too (Host / HostName / Port / User).

## Building from source

```sh
cd macos            # the macOS app lives here
brew install xcodegen
xcodegen generate
open BHTerminal.xcodeproj
```

Or package the signed `.dmg` + `.pkg` in one step: `bash scripts/package.sh`.

Then build/run in Xcode, or from the command line:

```sh
xcodebuild -scheme BHTerminal -configuration Release build
```

The project signs with a local self-signed **"BHTerminal Dev"** code-signing
certificate. To build on a fresh machine, create one in **Keychain Access →
Certificate Assistant → Create a Certificate** (Self-Signed Root, type: Code
Signing), or change `CODE_SIGN_IDENTITY` in `project.yml` to `-` (ad-hoc).

## Credits

- [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) — terminal emulator
- [Citadel](https://github.com/orlandos-nl/Citadel) — SFTP client
- [RoyalVNCKit](https://github.com/royalapplications/royalvnc) — VNC client

## License

© 2026 BiswasHost.
