# Security Policy

## Reporting a vulnerability

Please do not open a public GitHub issue for security vulnerabilities.

Instead, use [GitHub's private vulnerability reporting](https://github.com/MadMartian/rologlyphex/security/advisories/new) (Security tab → "Report a vulnerability"). This opens a private advisory visible only to the maintainers until a fix is ready.

## Scope

Rologlyphex is a local, single-user daemon: it grabs macropad function keys at the X11 level and synthesizes keystrokes into the focused window via XTest. Reports of interest include:

- Privilege escalation or ways to run code outside the invoking user's own session
- Input injection into windows other than the one currently focused
- Memory-safety issues in the `unsafe` FFI blocks wrapping Xlib/XTest calls
- Anything that lets a config file (`config.toml`, `layers.toml`) or client-socket input cause behavior beyond typing the requested character(s)

Rologlyphex has no network component and collects no data (see [PRIVACY.md](PRIVACY.md)), so reports about telemetry, data collection, or remote attack surface are out of scope.
