# BHTerminal — Linux port

> **Status: planned.** Placeholder.

A native Linux build of BHTerminal — the same MobaXterm-style workspace
(session tree + terminal + SFTP + VNC + tunnels) described in
[`../docs/PORTING.md`](../docs/PORTING.md), staying **config-compatible** with
the macOS app.

## Recommended shape

- **Terminal:** spawn the system `ssh` in a pty (inherits `~/.ssh/config`,
  agent, keys, ProxyJump). Render with **[VTE](https://gitlab.gnome.org/GNOME/vte)**
  (the GTK terminal widget) or `xterm.js`.
- **SFTP:** [libssh](https://www.libssh.org/) / paramiko / `ssh2` for the file
  browser.
- **VNC:** [gtk-vnc](https://gitlab.gnome.org/GNOME/gtk-vnc) or noVNC.
- **Credentials:** the **Secret Service** API via `libsecret` (GNOME Keyring /
  KWallet) — never the JSON config.
- **Tray + autostart:** `StatusNotifierItem` (AppIndicator) + a `.desktop`
  autostart entry.
- **UI framework:** GTK4 (native GNOME feel) or Qt; or Electron + xterm.js +
  `ssh2` for the fastest shared-UI route.

## Where to start

1. Read [`../docs/PORTING.md`](../docs/PORTING.md) — the §5 security rules
   (argument-injection guard, known-hosts fail-closed, secrets in the keyring)
   are **mandatory**.
2. Port the headless core first (data model, credential store, argv builder +
   validator, import parsers) with tests — translate from
   [`../macos/BHTerminal/`](../macos/BHTerminal/).
3. Then wire the terminal (pty), SFTP, VNC, tunnels, and the tray.

Packaging targets: Flatpak / AppImage / `.deb` + `.rpm`.
