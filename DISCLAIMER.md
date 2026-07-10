# Disclaimer

BHTerminal is an independent, open-source project by [BiswasHost](https://www.biswashost.com).

It is **not affiliated with, endorsed by, or derived from** MobaXterm (Mobatek),
Termius, or any other terminal/SSH product. Those names are trademarks of their
respective owners and are referenced here only to describe interoperability —
BHTerminal can **import session data** that users export from those tools. No
proprietary code or assets from those products are included in this repository.

BHTerminal builds on these open-source components, each under its own license:

- [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) — terminal emulator (MIT)
- [Citadel](https://github.com/orlandos-nl/Citadel) — SSH/SFTP (MIT)
- [RoyalVNCKit](https://github.com/royalapplications/royalvnc) — VNC client

The macOS app runs your system's own `/usr/bin/ssh` for terminal sessions, so it
inherits your existing SSH configuration, keys, and agent — BHTerminal does not
reimplement or bundle its own SSH transport for the terminal.

Use at your own risk. See [`LICENSE`](LICENSE) for warranty terms (there are none).
