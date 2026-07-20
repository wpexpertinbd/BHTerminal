<h1 align="center">BHTerminal</h1>

<p align="center">
  A free, open-source <b>SSH / SFTP / VNC client</b> for <b>macOS, Windows, and Linux</b>.<br>
  A MobaXterm/Termius-style workspace — session tree, terminal, file browser, and<br>
  remote desktops in one window. Imports your MobaXterm sessions.
</p>

<p align="center"><i>Built and maintained by <a href="https://www.biswashost.com">BiswasHost</a> 🇧🇩</i></p>

---

## Platforms

| OS | What it is | Status | Folder |
|----|-----------|--------|--------|
| 🍎 **macOS** | Native SwiftUI/AppKit app — SSH terminal (tabs + split panes), SFTP browser, VNC, tunnels, menu-bar mode. Signed `.dmg`/`.pkg`. | ✅ **shipping v1.0.0** | [`macos/`](macos/) |
| 🪟 **Windows** | Native shell (ConPTY + `ssh.exe`, SFTP, VNC) over the same feature spec | ⬜ planned | [`windows/`](windows/) |
| 🐧 **Linux** | Native shell (VTE/GTK, `ssh` in a pty, SFTP, VNC) | ⬜ planned | [`linux/`](linux/) |

**macOS users:** grab the latest `.dmg`/`.pkg` from the
[**latest release**](https://github.com/wpexpertinbd/BHTerminal/releases/latest)
and see [`macos/README.md`](macos/README.md).

## Features

- **Sessions sidebar** — folders + hosts, quick-connect filter, double-click to connect.
- **Real SSH terminal** — runs your system `ssh`, so `~/.ssh/config`, the agent,
  `ProxyJump`, and 2FA all work. Tabs + split panes, broadcast-input, snippets, session logging.
- **SFTP browser** — drag-and-drop transfer, and a path that **follows your terminal's directory**.
- **VNC** remote desktops in the same tabbed window.
- **SSH tunnels** — local / remote / dynamic, kept running in the background.
- **Menu-bar mode** — close the window and it keeps running (tunnels stay up).
- **Import from MobaXterm** (`.txt` passwords export or `.mxtsessions`) and `~/.ssh/config`.
- **Secure by design** — secrets only in the OS keychain, host-key verification, argument-injection hardened.

## How it works — one spec, native shells

A terminal client has no single portable core: SSH, SFTP, VNC, PTY, credential
storage, and UI each want a different best-in-class library per OS. So the shared
thing here isn't code — it's the **contract**:

- **[`docs/PORTING.md`](docs/PORTING.md)** — the OS-neutral spec: the feature set,
  the config-compatible data model, and the **mandatory security rules** every
  port must pass. **The contract.**
- **[`macos/`](macos/)** — the reference implementation (Swift). Ports translate
  its headless core (data model, credential store, argv validator, import
  parsers) then wire a native shell.

Every port stays **config-compatible** — the same `sessions.json` model — so your
sessions move between platforms.

> Porting to a new OS? Read [`docs/PORTING.md`](docs/PORTING.md), build the
> headless core first (with tests), then wire the terminal (`ssh` in a pty),
> SFTP, VNC, tunnels, and the tray.

## Repository layout
```
.
├── docs/PORTING.md   # shared spec + security contract for all ports
├── macos/            # shipping macOS app (SwiftUI/AppKit) + installers
├── windows/          # Windows port (planned)
├── linux/            # Linux port (planned)
├── LICENSE           # MIT
├── SECURITY.md
└── DISCLAIMER.md     # not affiliated with MobaXterm / Termius
```

## License
MIT — see [`LICENSE`](LICENSE). Not affiliated with, endorsed by, or derived from
MobaXterm, Termius, or any other product — see [`DISCLAIMER.md`](DISCLAIMER.md).

## ☕ Support

This project is free and open-source. If it made your server work easier, you can
**buy me a coffee** — it genuinely helps me keep building and maintaining free
tools like this. 🙏

- **bKash/Nagad** (Personal · *Send Money*): **`01710378396`**

ধন্যবাদ! / Thank you!

<p align="center">Made with care by <a href="https://www.biswashost.com">BiswasHost</a> 🇧🇩</p>
