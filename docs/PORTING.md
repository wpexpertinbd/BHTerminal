# BHTerminal — Porting Guide & Feature Contract

This is the OS-neutral **contract** for BHTerminal. The macOS app
([`../macos/`](../macos/)) is the reference implementation; the Windows and
Linux ports are native apps that provide the same features and stay
**config-compatible** with it.

Unlike a keyboard engine, a terminal client has no single portable core — SSH,
SFTP, VNC, PTY, credential storage, and UI all have different best-in-class
libraries per OS. So the shared thing here is not code, it's **this spec**: the
feature set, the data model, and the security rules every port must honor.

---

## 1. What BHTerminal is

A MobaXterm/Termius-style workspace in one window:

- A **session tree** (folders + hosts) on the left.
- A **terminal** (tabs + split panes) on the right.
- An **SFTP file browser** that follows the terminal's working directory.
- **VNC** remote-desktop tabs in the same window.
- **SSH tunnels** (local / remote / dynamic) that keep running in the background.
- Import from **MobaXterm** and **OpenSSH config**.

## 2. The data model (config-compatible across ports)

All ports read/write the same logical model so a user's sessions are portable.

**`Host`**
| field | type | notes |
|-------|------|-------|
| `id` | UUID | stable identity; also the Keychain/credential key |
| `name` | string | display name |
| `hostname` | string | **must pass §5 validation** |
| `port` | int | 22 (ssh) / 5900 (vnc) default |
| `username` | string | **must pass §5 validation** |
| `authMethod` | `password` \| `privateKey(path)` \| `agent` | |
| `connectionType` | `ssh` \| `vnc` | |
| `jumpHostID` | UUID? | ProxyJump |
| `folderID` | UUID? | tree placement |
| `tunnels` | `[TunnelRule]` | |
| `vncSharedClipboard` | bool | default **false** |

Secrets (password, passphrase) are **never** in this model — they live in the
OS credential store keyed by `host.<id>`. The model persists as JSON
(`sessions.json` on macOS). Keep the field names identical so exported configs
move between platforms.

## 3. Subsystems and their per-OS equivalents

| Subsystem | macOS (reference) | Windows | Linux |
|-----------|-------------------|---------|-------|
| Terminal emulator view | [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) | [xterm.js] (Electron) / ConPTY + a native widget / [WinUI](https://learn.microsoft.com/windows/apps/winui/) | [VTE](https://gitlab.gnome.org/GNOME/vte) (GTK) / xterm.js (Electron) |
| Interactive SSH | system `/usr/bin/ssh` in a pty | `ssh.exe` (OpenSSH for Windows) via ConPTY, or [libssh2] | system `ssh` in a pty |
| SFTP (structured) | [Citadel](https://github.com/orlandos-nl/Citadel) | [SSH.NET] / libssh2 / `ssh2` (Node) | [libssh]/paramiko/`ssh2` |
| VNC | [RoyalVNCKit](https://github.com/royalapplications/royalvnc) | [noVNC]/native RFB | [gtk-vnc]/noVNC |
| Credential store | Keychain | Windows Credential Manager (DPAPI) | Secret Service / libsecret (GNOME Keyring / KWallet) |
| Tunnels | background `ssh -N -L/-R/-D` Process | same, with `ssh.exe` | same |
| known_hosts verify | `~/.ssh/known_hosts` | `%USERPROFILE%\.ssh\known_hosts` | `~/.ssh/known_hosts` |
| Launch at login | `SMAppService` | Startup registry / Task Scheduler | `.desktop` autostart |
| Menu-bar / tray | `MenuBarExtra` | system tray (NotifyIcon) | `StatusNotifierItem` (AppIndicator) |

**Recommended reuse of the terminal:** just like macOS, spawn the platform's own
`ssh` binary in a pty (ConPTY on Windows, a pty on Linux). You inherit the user's
SSH config, agent, keys, ProxyJump, and 2FA for free, and you don't reimplement
crypto. Use a dedicated SFTP library only for the file browser.

## 4. UX contract (what "the same app" means)

- Double-clicking a host connects the terminal **and** the SFTP pane together.
- The SFTP breadcrumb follows the terminal's cwd (emit/parse **OSC 7**).
- Terminal tabs + split panes; broadcast-input to all panes in a tab; snippets.
- Closing the window keeps the app running in the tray/menu-bar; tunnels survive.
- Import: **MobaXterm passwords `.txt`** (host+port+user+password → credential
  store), **MobaXterm `.mxtsessions`** (names+folders, no passwords), and
  **`~/.ssh/config`**. See [`../macos/README.md`](../macos/README.md#migrating-from-mobaxterm).

## 5. Security rules — MANDATORY for every port

These are not optional; they're why BHTerminal is safe to hand your servers to.

1. **Secrets only in the OS credential store**, never in the JSON config or logs.
2. **known_hosts verification, fail-closed** for the structured-SSH/SFTP path.
   Never trust-on-first-use silently.
3. **Argument-injection guard.** Before building any `ssh` argv, validate
   `hostname` and `username` against an allow-list (letters, digits, and
   `.-_:%[]` for hosts; `.-_` for users; no leading `-`, no whitespace/shell
   metacharacters) and reject the rest. End option parsing with `--` before the
   destination operand. A host named `-oProxyCommand=…` must never reach `ssh`.
4. **Imports are untrusted.** Re-validate every hostname/username at import;
   clamp/strip imported names; size-cap the import file.
5. **VNC clipboard sharing defaults off** (opt-in per host).
6. Ship with the platform's exploit-mitigation defaults (Hardened Runtime on
   macOS; equivalent on others).

## 6. How to start a port

1. Read this file end-to-end.
2. Build the **headless core** first: the data model (§2), the credential-store
   wrapper, the argv builder + validator (§5.3), and the import parsers (§4) —
   with unit tests, no UI. The macOS versions in
   [`../macos/BHTerminal/`](../macos/BHTerminal/) (`Models/`, `Persistence/`,
   `Utilities/SSHSafety.swift`, `SSH/KnownHostsValidator.swift`, `Import/`) are
   the reference to translate.
3. Wire the terminal (spawn `ssh` in a pty), then the SFTP pane, then VNC, then
   tunnels, then the tray + launch-at-login.
4. Verify against a real host — this is an app whose whole value is "does it
   actually SSH/SFTP/VNC correctly."

Framework choice is yours (native WinUI/GTK/Qt for the smallest, best-feeling
app; or Electron+xterm.js+ssh2 for the fastest shared-UI route). Keep the core
a clean, testable module regardless.
