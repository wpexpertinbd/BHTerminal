# BHTerminal — Windows port

> **Status: planned.** Placeholder.

A native Windows build of BHTerminal — the same MobaXterm-style workspace
(session tree + terminal + SFTP + VNC + tunnels) described in
[`../docs/PORTING.md`](../docs/PORTING.md), staying **config-compatible** with
the macOS app.

## Recommended shape

- **Terminal:** spawn Windows OpenSSH (`ssh.exe`) over **ConPTY** — you inherit
  the user's `~/.ssh/config`, agent, keys, and ProxyJump, same as the macOS app
  reuses `/usr/bin/ssh`. Render with a native terminal control or `xterm.js`.
- **SFTP:** [SSH.NET](https://github.com/sshnet/SSH.NET) (or `libssh2`) for the
  structured file browser.
- **VNC:** a native RFB client or [noVNC].
- **Credentials:** Windows **Credential Manager** (DPAPI) — never the JSON config.
- **Tray + launch-at-login:** `NotifyIcon` + Startup registration.
- **UI framework:** WinUI 3 / WPF for a native feel, or Electron + xterm.js +
  `ssh2` for the fastest shared-UI route.

## Where to start

1. Read [`../docs/PORTING.md`](../docs/PORTING.md) (the feature + security
   contract) and §5 in particular — the argument-injection and known-hosts rules
   are **mandatory**.
2. Port the headless core first (data model, credential store, argv builder +
   validator, import parsers) with tests — translate from
   [`../macos/BHTerminal/`](../macos/BHTerminal/).
3. Then wire the terminal (ConPTY), SFTP, VNC, tunnels, and the tray.
