# Rologlyphex

![Demo](demo.webp)

A Rust GTK4 overlay daemon and Unicode input synthesizer for macropads. Rotary knob cycles through layout decks, displaying a floating overlay with the current layout name and button legends. Button presses type Unicode characters into the focused window via XTest.

Named after Rolodex + glyphs.

## Features

- **Layout overlay** — floating, click-through notification window shows the active layout and per-button characters when the knob cycles layouts
- **Unicode input synthesis** — types characters into the focused X11 window using XTest + keyboard remapping (no xdotool dependency)
- **keyd integration** — subscribes to keyd's IPC socket for real-time layout change events; parses keyd config for button legends
- **Config file** — optional `~/.config/rologlyphex/config.toml` for persistent settings; CLI flags override file values
- **Hot reload** — picks up keyd config changes via inotify without restarting
- **Agent-Ready Documentation** — Following Spec-Driven Development (SDD), all technical specs and behavioral contracts live in `sdd/`. `AGENTS.md` provides an entry point for AI contributors.

## Requirements

- Linux / X11 (**Wayland is not supported** — set `GDK_BACKEND=x11` if your session defaults to Wayland)
- GTK4 (`libgtk-4-dev`)
- D-Bus (`libdbus-1-dev`)
- [keyd](https://github.com/rvaiya/keyd) v2.5+
- Rust toolchain (for building)

## Install

```bash
make install
systemctl --user enable --now rologlyphex
```

This builds the release binary, installs it to `/usr/local/bin/`, and installs the systemd user service.

Your user must be in the `keyd` group to access the keyd IPC socket:

```bash
sudo usermod -aG keyd $USER
# Log out and back in for group membership to take effect
```

## Usage

### Daemon mode (default)

```
rologlyphex [-c <path>] [-t <ms>] [-s <WxH>] [-v]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config <path>` | (config file or required) | Path to keyd config file |
| `-t, --timeout <ms>` | 3000 | Overlay auto-dismiss timeout |
| `-s, --size <WxH>` | 600x275 | Overlay window size |
| `-v, --verbose` | off | Enable debug logging |

All flags can be set in `~/.config/rologlyphex/config.toml` instead:

```toml
keyd_config = "/etc/keyd/macropad.conf"
timeout = 3000
size = "600x275"
verbose = false
```

CLI flags always override config file values. A missing config file is not an error.

### Client mode

```
rologlyphex type <char>
```

Sends a Unicode character to the running daemon for input synthesis. Used in keyd config bindings:

```ini
f13 = command(rologlyphex type →)
f14 = command(rologlyphex type 🔥)
```

> **Note**: emoji and other non-BMP characters (U+10000+) produce wrong output in JetBrains IDEs (RustRover, IntelliJ, etc.) due to Java/AWT keysym truncation. They work correctly in terminals, browsers, and GTK/Qt apps.

## How it works

### Overlay

The daemon connects to keyd's IPC socket (`/var/run/keyd.socket`) and listens for layout change events. When the active layout changes, a GTK4 window appears in the top-right corner of the rightmost monitor showing the layout name and button legends. The window uses `_NET_WM_WINDOW_TYPE_NOTIFICATION` and an empty input region so it never steals focus or intercepts clicks.

Long layout names auto-reduce to 2/3 font size if they would overflow the window, with ellipsis truncation as a final fallback.

### Unicode input

Characters are typed via X11's XTest extension. Each Unicode codepoint is mapped to a keysym (`0x01000000 + codepoint` for U+0100+) and bound to an unused keycode via `XChangeKeyboardMapping`. Mappings are cached permanently, so only the first press of each unique character incurs a ~250ms delay (due to `MappingNotify` broadcast). Subsequent presses are sub-millisecond.

### Config parsing

The daemon parses keyd's config format to extract layout metadata:

- **Layout names** from `[name:layout]` section headers
- **Display labels** from `# label: <text>` comments preceding a section header (falls back to snake_case to Title Case conversion)
- **Button legends** from non-`setlayout()` bindings, with `command(rologlyphex type ...)` and `macro(...)` wrappers stripped

Config is re-parsed automatically on file changes (inotify, debounced 200ms) and on keyd reloads.

## Architecture

```
src/
  main.rs       CLI parsing, config file loading, daemon startup, GTK application
  settings.rs   App config file (config.toml) loading and CLI merge
  overlay.rs    GTK4 overlay window, direct Xlib FFI for WM properties
  xtype.rs      XTest input synthesis, keysym caching, keyboard remapping
  server.rs     Unix socket listener for type commands
  client.rs     Unix socket client for `rologlyphex type`
  socket.rs     Socket path resolution (D-Bus seat detection, XDG_RUNTIME_DIR, root fallback)
  config.rs     keyd config parser, layout/button legend extraction
  ipc.rs        keyd IPC subscription, inotify config watcher
```

The daemon runs 4 concurrent activities:

1. **GTK main loop** (main thread) — overlay window, 100ms layout-change polling
2. **keyd IPC listener** (thread) — subscribes to layout events, reconnects on keyd restart
3. **Socket server** (thread) — accepts `type` commands, synthesizes keypresses via XTest
4. **inotify watcher** (thread) — monitors keyd config for changes

## Uninstall

```bash
systemctl --user disable --now rologlyphex
make uninstall
```

## License

Part of the [Macropad](../) project.
