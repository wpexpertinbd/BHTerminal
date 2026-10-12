# BHTerminal on Linux

**This shipped.** Linux is no longer a planned port: since **v2.0.0** the app is
built from one source for macOS, Windows and Linux — a Rust core in
[`../crates/bhterm-core`](../crates/bhterm-core) and a Tauri shell in
[`../app`](../app) — so there is no separate Linux tree to write.

Download the `.deb` or `.rpm` for your architecture (`amd64`/`x86_64` or
`arm64`/`aarch64`) from the
[latest release](https://github.com/wpexpertinbd/BHTerminal/releases/latest).

## Requirements

The webview is the system's: **WebKitGTK 4.1**. On Debian and Ubuntu that is
`libwebkit2gtk-4.1-0`, which the `.deb` depends on; on Fedora it is
`webkit2gtk4.1`. Everything else — the terminal, SFTP and the forwards — uses
the `ssh` already on your machine, so your `~/.ssh/config`, your agent,
`ProxyJump` and 2FA all work as they do in a shell.

An arm64 build is published alongside amd64 because a Linux VM on an
Apple-silicon Mac is arm64, and an amd64 package simply refuses to install
there.

What this folder used to hold — the spec and the security rules a port had to
meet — lives in [`../docs/PORTING.md`](../docs/PORTING.md), which is still the
contract for the behaviour.
