# Development Policy

## Build

- Build with `cargo build --release` for all testing and deployment
- The binary must compile with zero errors on stable Rust
- Compiler warnings should be resolved, not suppressed (except temporary `dead_code` for shelved features)
- Install via `make install`, which builds, copies the binary to `/usr/local/bin/`, and installs the systemd user service

## Testing

### Unit tests

- The layers config parser (`layers.rs`) must have unit tests covering:
  - Alias resolution: `[keys]` physical→logical mapping; typing keyed by logical name; physical-key references resolving to their logical name
  - Navigation: `[navigation]` raw F-keys, defaults (F16/F18), the disjointness rule (a key may not be both a navigation key and a `[keys]` alias), and distinct prev/next
  - Grouping: global `[[groups]]` applied per layer in render order; ungrouped keys forming the anonymous group first
  - Validation: unknown `layer_order` entries skipped; empty ring rejected; default-label title-casing
- The pure overlay-positioning logic (`overlay.rs` `Corner`) must have unit tests for parse and per-corner placement.
- Run with `cargo test`

### Manual integration testing

- After any change to the overlay, key-grab input, or typing code, verify end-to-end by:
  1. Building and (re)starting the daemon (foreground `cargo run -- -v` for direct control, or via systemd)
  2. Cycling through all layers with the knob -- overlay must track the knob in real time, and navigation must not cause whole-desktop sluggishness (ANTI-PATTERNS #21)
  3. Pressing buttons on at least two layers (both macropad and secondary-keyboard keys) -- the glyph must appear in the focused window on key release, reliably across repeated presses
  4. Editing `layers.toml` and restarting the daemon -- the new glyph map must take effect

## Input and output sanitization

All data crossing system boundaries must be validated:

- **Minimum and maximum length**: every input and output must have a defined minimum and maximum length, enforced at the point of ingress or egress
- **Client socket**: the `type` command accepts exactly one Unicode character (1-4 bytes UTF-8). The server enforces a 16-byte read limit and extracts only the first character, logging a warning if extra characters are received
- **Grabbed keys**: only the configured function keys (F13–F24, minus the navigation keys) are grabbed; unknown keycodes/events are ignored
- **CLI arguments**: validated at parse time with clear error messages for missing, malformed, or out-of-range values (e.g. `--size` must have positive dimensions)
- **Config files**: the daemon loads `~/.config/rologlyphex/config.toml` (settings) and `~/.config/rologlyphex/layers.toml` (glyph map) at startup. CLI arguments override config.toml values; a missing config.toml is not an error, but a missing/invalid layers.toml is fatal. Unknown or out-of-range references in layers.toml are skipped with a warning (see SCHEMA.md validation)

## Code style

- No external linter or formatter is enforced; follow the existing code style
- Prefer explicit error messages over panics -- use `Result` at module boundaries
- Use the `debug_log!` macro for all debug output (gated behind `--verbose` / `-v`)
- Keep `unsafe` blocks minimal and adjacent to the FFI call they wrap
- Do not add dependencies without justification; prefer stdlib and existing crates
- Source files that exceed 500 lines are discouraged; notify the operator and refactor if approved

## Dependencies

- Do not add crates for functionality achievable with existing dependencies or stdlib
- The `x11` crate provides Xlib and XTest bindings -- do not add separate X11 binding crates
- GTK4 bindings (`gtk4`, `gdk4`, `glib`, `cairo-rs`) are the UI framework -- do not introduce alternative UI toolkits
- Input comes from grabbing the device function keys at the X11 level (`XGrabKey`), not from keyd IPC or filesystem watching -- do not reintroduce keyd, an IPC dependency, or file-watching crates (see ANTI-PATTERNS #20, and the keyd-retirement history in TECH.md)

## Architecture rules

- The daemon is a single binary with two modes (daemon and client), dispatched by CLI arguments
- Overlay window properties must be set via direct Xlib calls, never via external commands (`xdotool`, `xprop`, `wmctrl`)
- Input synthesis must use XTest via Xlib, never by shelling out to `xdotool`
- The overlay must never steal focus or intercept input events
- The key-grab and socket-server threads must not block or interfere with the GTK main loop, and each must use its own Xlib `Display` (never share a connection across threads)
- Keymap changes (`XChangeKeyboardMapping`) must be batched into a single call per layer change and never issued per keypress or on navigation (ANTI-PATTERNS #21); injection via `XTest` must happen on key release, not press (ANTI-PATTERNS #22)
- All layer metadata and glyphs come from `layers.toml`; physical function keys (F13–F24) must not appear in runtime state or logging beyond the `[keys]`/`[navigation]` config and the grab boundary
- The application requires an X11 display server; Wayland is not supported. The daemon detects the GDK backend at startup and exits with a clear error if X11 is not active

## Deployment

- The systemd user service file (`rologlyphex.service`) must be kept in sync with CLI flag changes (it must not pass a keyd config path; the daemon now loads `layers.toml`)
- The Makefile `install` target is the canonical install method
- The daemon runs as a normal user-session X client -- no root, no `input`-group membership, no keyd group is required (the X server delivers the grabbed F13–F24 keys directly)

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
