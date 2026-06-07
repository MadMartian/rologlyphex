# Development Policy

## Build

- Build with `cargo build --release` for all testing and deployment
- The binary must compile with zero errors on stable Rust
- Compiler warnings should be resolved, not suppressed (except temporary `dead_code` for shelved features)
- Install via `make install`, which builds, copies the binary to `/usr/local/bin/`, and installs the systemd user service

## Testing

### Unit tests

- The config parser (`config.rs`) must have unit tests covering:
  - Layout header detection (valid and invalid formats, including false-positive suffixes)
  - Label comment parsing (`# label:` present, absent, non-adjacent)
  - Identifier-to-label derivation (snake_case to Title Case)
  - Display character extraction from `macro()`, `macro2()`, bare unicode, `command(rologlyphex type ...)`, and `command(xdotool type ...)`
  - `setlayout()` and `noop` binding exclusion
  - Per-button label override via `# label:` comment (single-char extraction, extra chars warning)
- Run with `cargo test`

### Manual integration testing

- After any change to overlay, IPC, or input synthesis code, verify end-to-end by:
  1. Building and installing the binary
  2. Restarting the daemon (via `trouble-run.sh` or systemd)
  3. Cycling through all layouts with the knob -- overlay must appear with correct content
  4. Pressing buttons on at least two layouts -- characters must appear in the focused window
  5. Editing the keyd config and running `keyd reload` -- overlay must reflect changes without daemon restart

## Input and output sanitization

All data crossing system boundaries must be validated:

- **Minimum and maximum length**: every input and output must have a defined minimum and maximum length, enforced at the point of ingress or egress
- **Client socket**: the `type` command accepts exactly one Unicode character (1-4 bytes UTF-8). The server enforces a 16-byte read limit and extracts only the first character, logging a warning if extra characters are received
- **keyd IPC**: incoming data is buffered and processed only as complete newline-terminated lines; partial reads are retained for the next read cycle
- **CLI arguments**: validated at parse time with clear error messages for missing, malformed, or out-of-range values (e.g. `--size` must have positive dimensions)
- **Config file**: the daemon loads settings from `~/.config/rologlyphex/config.toml` (or `$XDG_CONFIG_HOME/rologlyphex/config.toml`) at startup. CLI arguments always override values found in the config file. A missing config file is not an error.
- **Config parsing**: layout section headers must match `[name:layout]` exactly (not partial matches like `[name:layout-extra]`)

## Code style

- No external linter or formatter is enforced; follow the existing code style
- Prefer explicit error messages over panics -- use `Result` at module boundaries
- Use the `debug_log!` macro for all debug output (gated behind `--verbose` / `-v`)
- Keep `unsafe` blocks minimal and adjacent to the FFI call they wrap
- Do not add dependencies without justification; prefer stdlib and existing crates

## Dependencies

- Do not add crates for functionality achievable with existing dependencies or stdlib
- The `x11` crate provides Xlib and XTest bindings -- do not add separate X11 binding crates
- GTK4 bindings (`gtk4`, `gdk4`, `glib`, `cairo-rs`) are the UI framework -- do not introduce alternative UI toolkits
- The `notify` crate handles inotify -- do not add alternative file-watching crates

## Architecture rules

- The daemon is a single binary with two modes (daemon and client), dispatched by CLI arguments
- Overlay window properties must be set via direct Xlib calls, never via external commands (`xdotool`, `xprop`, `wmctrl`)
- Input synthesis must use XTest via Xlib, never by shelling out to `xdotool`
- The overlay must never steal focus or intercept input events
- The socket server thread must not block or interfere with the GTK main loop
- Config parsing must derive all layout metadata from the keyd config file -- no separate metadata files
- The application requires an X11 display server; Wayland is not supported. The daemon detects the GDK backend at startup and exits with a clear error if X11 is not active

## Deployment

- The systemd user service file (`rologlyphex.service`) must be kept in sync with CLI flag changes
- The Makefile `install` target is the canonical install method
- The user must be in the `keyd` group for IPC access; this is a documented prerequisite, not something the application should attempt to fix at runtime

## Privacy and attribution

- Do not write specific hardware brand names, product model numbers, or trademarks in any file in this repository
- Do not include machine-specific details (hardware identifiers, vendor IDs, product IDs, device-specific configurations) in any file in this repository
- Describe hardware generically (e.g., "macropad", "secondary keyboard", "full-keyboard device") rather than by brand or model
- When documenting bugs or anti-patterns, describe the class of software or hardware involved, not the specific product that exhibited the behavior

## Debug output

- All debug logging uses the `debug_log!` macro, which is gated behind `--verbose` / `-v`
- Debug output is disabled by default; enable with `--verbose` for troubleshooting
- Never log to files; rely on systemd journal capture via `StandardError=journal`
- Error-level messages (connection failures, lock poisoning, invalid input) always print via `eprintln!` regardless of verbose flag
