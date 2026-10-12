# BHTerminal on Windows

**This shipped.** Windows is no longer a planned port: since **v2.0.0** the app
is built from one source for macOS, Windows and Linux — a Rust core in
[`../crates/bhterm-core`](../crates/bhterm-core) and a Tauri shell in
[`../app`](../app) — so there is no separate Windows tree to write.

Download `BHTerminal_<version>_x64-setup.exe` (or `_arm64-setup.exe`) from the
[latest release](https://github.com/wpexpertinbd/BHTerminal/releases/latest).
There is deliberately no `.msi`.

## The one real difference on Windows

Windows OpenSSH does not implement connection sharing (`ControlMaster`), so the
file browser opens its own SSH connection instead of riding the terminal's
login. A saved password is filled in for you, so it stays invisible — but the
app tells you this is happening rather than behaving differently in silence.
The single switch for it is `ssh::multiplexing_supported()` in the core.

What this folder used to hold — the spec and the security rules a port had to
meet — lives in [`../docs/PORTING.md`](../docs/PORTING.md), which is still the
contract for the behaviour.
