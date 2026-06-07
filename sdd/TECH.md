# Technical Architecture

## System overview

```
┌─────────────────┐     USB HID      ┌──────────┐    evdev     ┌──────────┐
│    Macropad     │ ───────────────> │  Linux   │ ──────────> │   keyd   │
│  (keys + knob)  │    F13-F18       │  kernel  │  f13-f18    │  daemon  │
└─────────────────┘                  └──────────┘             └────┬─────┘
                                                                   │
                                            ┌──────────────────────┼────────────────┐
                                            │                      │                │
                                   command(rolo type X)    setlayout(name)    IPC /name events
                                            │                      │                │
                                            v                      v                v
                                    ┌──────────────┐     ┌──────────────┐   ┌──────────────┐
                                    │  rologlyphex  │     │    keyd      │   │  rologlyphex  │
                                    │  type <char>  │     │  (internal)  │   │   IPC thread  │
                                    │  (CLI client) │     └──────────────┘   └───────┬───────┘
                                    └───────┬───────┘                                │
                                            │ Unix socket                   layout name
                                            v                                       │
                                    ┌──────────────┐                                 v
                                    │  rologlyphex  │                        ┌──────────────┐
                                    │ socket server │                        │   GTK main   │
                                    │   thread      │                        │    thread    │
                                    └───────┬───────┘                        └───────┬──────┘
                                            │                                       │
                                     XTest keypress                          show/hide overlay
                                            │                                       │
                                            v                                       v
                                    ┌──────────────┐                        ┌──────────────┐
                                    │   X server   │                        │  GTK4 window │
                                    │              │ <───────────────────── │  (Xlib FFI)  │
                                    └──────────────┘    WM properties       └──────────────┘
```

## Third-party components

### Rust crates

| Crate | Version | Role |
|-------|---------|------|
| `gtk4` | 0.9 | GTK4 bindings -- application lifecycle, window management, widget tree, CSS styling |
| `gdk4` | 0.9 | GDK4 bindings -- display/monitor enumeration, surface access for X11 FFI |
| `glib` | 0.20 | GLib bindings -- main loop integration, timers (`timeout_add_local`), idle callbacks |
| `cairo-rs` | 0.20 | Cairo bindings -- creating empty input regions for click-through windows |
| `x11` | 2.21 | Raw Xlib/XTest/keysym bindings -- keyboard remapping, synthetic key events, window properties, clipboard selection |
| `notify` | 6.1 | Cross-platform file watcher (inotify on Linux) -- monitors keyd config for changes |
| `libc` | 0.2 | POSIX type definitions -- `getuid()` for socket path resolution |
| `dbus` | 0.9 | D-Bus client -- queries `org.freedesktop.login1` to identify the active seat0 X11 session for root-invoked socket discovery |
| `serde` | 1 | Serialization framework -- used for config file deserialization |
| `toml` | 0.8 | TOML parser -- used for config file parsing |

### System dependencies

| Component | Role |
|-----------|------|
| GTK4 (`libgtk-4-dev`) | UI toolkit runtime; provides window rendering, CSS engine, Pango text layout |
| X11 server | Display server; receives XTest key events, manages window properties. **Required** -- Wayland is not supported |
| keyd | Key remapping daemon; intercepts macropad input, provides IPC for layout events |
| systemd (user) | Service manager; launches daemon at login, captures logs to journal |

**X11 requirement**: The daemon uses X11-specific APIs (XTest for input synthesis, Xlib for window properties, GDK X11 backend for native handles). At startup, the daemon detects the GDK backend and exits with a clear error if X11 is not active. On systems defaulting to Wayland, set `GDK_BACKEND=x11` before launching.

### FFI boundaries

The application crosses two FFI boundaries not covered by crate bindings:

1. **GDK4 X11 backend** -- `gdk_x11_surface_get_xid()` and `gdk_x11_display_get_xdisplay()` are declared as `extern "C"` in `overlay.rs` to obtain native X11 handles from GDK4 surfaces. These are used to set WM properties (`_NET_WM_WINDOW_TYPE`, `_NET_WM_STATE`) and position the window via `XMoveWindow`.

2. **keyd IPC protocol** -- The daemon sends a raw `IpcMessage` struct (4112 bytes: `i32` type + `u32` timeout + `[u8; 4096]` data + `usize` size) over a Unix socket to `/var/run/keyd.socket`. This matches keyd's `struct ipc_message` from its C header. Layout events arrive as ASCII lines prefixed with `/`.

## Module responsibilities

| Module | Thread | Responsibility |
|--------|--------|----------------|
| `main.rs` | main | CLI parsing, GTK `Application` setup, thread spawning, 100ms layout-change polling, panic hook for crash logging |
| `settings.rs` | main | App config file loading, CLI merge, settings resolution |
| `overlay.rs` | main | GTK4 window lifecycle, Xlib FFI for WM properties, CSS styling, font-size fallback, FlowBox legend wrapping, content-driven window height, dismiss timer |
| `xtype.rs` | socket server | XTest key synthesis, `XChangeKeyboardMapping` for unmapped keysyms, keysym cache |
| `server.rs` | socket server | Unix socket listener, reads character from client, delegates to `XTyper` |
| `client.rs` | (separate process) | Connects to daemon socket, sends character, exits |
| `socket.rs` | any | Socket path resolution (`$XDG_RUNTIME_DIR` or `/run/user/*/` scan) |
| `config.rs` | IPC / inotify | keyd config parser, layout/button legend extraction, label resolution |
| `ipc.rs` | IPC + inotify | keyd socket subscription (`IPC_LAYER_LISTEN`), reconnection, inotify config watcher with debounce |

## Concurrency model

The daemon runs four concurrent activities:

1. **GTK main loop** (main thread) -- owns the overlay window, polls a `Mutex<String>` every 100ms for layout changes
2. **keyd IPC listener** (spawned thread) -- holds a persistent connection to keyd's Unix socket, writes layout name to the shared `Mutex<String>`, reconnects on socket closure
3. **Socket server** (spawned thread) -- accepts connections on the `rologlyphex.sock` Unix socket, reads characters, calls `XTyper::type_str()` on its own Xlib `Display` connection
4. **inotify watcher** (spawned thread) -- monitors the keyd config file, triggers re-parse into the shared `RwLock<HashMap<String, LayoutInfo>>`

The GTK thread communicates with the IPC thread via `Arc<Mutex<String>>` (current layout name). Layout metadata is shared via `Arc<RwLock<HashMap<String, LayoutInfo>>>`, written by the IPC and inotify threads, read by the GTK thread. The IPC thread signals keyd reload events to the socket server thread via `Arc<AtomicBool>` (reload flag); the socket server checks and clears this flag before each `type_char()` call, triggering an `XTyper::rescan()` when set.

The XTyper in the socket server thread has its own independent Xlib `Display` connection, separate from GTK's. This avoids thread-safety issues with Xlib (which is not thread-safe by default).
