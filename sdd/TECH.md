# Technical Architecture

## System overview

rologlyphex owns the macropad experience end-to-end: it grabs the device function keys at
the X11 level, runs the layer-ring state machine, and types each layer's glyph via XTest.
keyd has been retired; rologlyphex owns input, layers, typing, and UI end-to-end.

Most glyphs type via the keysym path. Non-BMP glyphs (emoji, U+10000+) destined for a focused
Java/AWT window are the exception: AWT truncates non-BMP keysyms, so those are delivered via a
focus-gated clipboard paste served by a dedicated owner thread (see *Non-BMP clipboard
routing* below).

```
┌────────────────────┐  USB HID: F13–F18                ┌────────────────────────────────┐
│      macropad      │ ───────────────────────────────> │                                │
│  (3 buttons + knob)│                                  │          X server              │
└────────────────────┘                                  │                                │
┌────────────────────┐  mgmt daemon: F19–F24            │   F13–F24 Key{Press,Release}   │
│ secondary keyboard │ ───────────────────────────────> │              │                 │
│   (6 macro keys)   │                                  └──────────────┼─────────────────┘
└────────────────────┘                                    passive grab │
                                                                       v
                    ┌────────────────────────────────────────────────────────────────────┐
                    │                       rologlyphex daemon                           │
                    │  ┌──────────────────┐        ┌──────────────────────────────────┐  │
                    │  │  key-grab thread │ layer  │        GTK main loop             │  │
                    │  │  XGrabKey F13–24 │ name   │  overlay window + "Please Wait"  │  │
                    │  │  layer ring      │───────>│  100ms poll (Mutex<String>)      │  │
                    │  │  LayerTyper      │ flags  └───────────────┬──────────────────┘  │
                    │  └────────┬─────────┘                        │ Xlib FFI            │
                    │   XTest   │                                  v  (WM properties)    │
                    │           │                          ┌───────────────┐             │
                    │  ┌────────┼──────────┐               │  GTK4 windows │             │
                    │  │ socket server     │ (manual       └───────────────┘             │
                    │  │  `type` / `show`  │  CLI use)                                   │
                    │  └───────────────────┘                                             │
                    └───────────│────────────────────────────────────────────────────────┘
                                v
                          focused window (synthetic keystrokes)
```

## Third-party components

### Rust crates

| Crate | Version | Role |
|-------|---------|------|
| `gtk4` | 0.9 | GTK4 bindings -- application lifecycle, window management, widget tree, CSS styling |
| `gdk4` | 0.9 | GDK4 bindings -- display/monitor enumeration, surface access for X11 FFI |
| `glib` | 0.20 | GLib bindings -- main loop integration, timers (`timeout_add_local`), signal handlers |
| `cairo-rs` | 0.20 | Cairo bindings -- creating empty input regions for click-through windows |
| `x11` | 2.21 | Raw Xlib/XTest/keysym bindings -- key grabbing, keyboard remapping, synthetic key events, window properties, error handlers |
| `libc` | 0.2 | POSIX -- `getuid()` for socket path resolution; `poll()` on the X connection fd in the key-grab loop |
| `serde` | 1 | Serialization framework -- config file deserialization |
| `toml` | 0.8 | TOML parser -- `config.toml` and `layers.toml` |

### System dependencies

| Component | Role |
|-----------|------|
| GTK4 (`libgtk-4-dev`) | UI toolkit runtime; window rendering, CSS engine, Pango text layout |
| X11 server | Display server; receives the grabbed keys, XTest events, manages window properties. **Required** -- Wayland is not supported |
| systemd (user) | Service manager; launches the daemon at login, captures logs to journal |

**Input sources** are environment-specific and not rologlyphex dependencies: the macropad is
flashed (via `ch57x-keyboard-tool`) to emit F13–F18 in hardware, and a secondary keyboard's
macro keys are mapped to F19–F24 by a keyboard-management daemon. rologlyphex only requires
that the keys arrive at the X server as F13–F24; it grabs them regardless of source.

**X11 requirement**: The daemon uses X11-specific APIs (XGrabKey, XTest, Xlib window
properties, GDK X11 backend). At startup it detects the GDK backend and exits with a clear
error if X11 is not active. On Wayland-default systems, set `GDK_BACKEND=x11`.

### FFI boundaries

The application crosses two FFI boundaries not covered by crate bindings:

1. **GDK4 X11 backend** -- `gdk_x11_surface_get_xid()` and `gdk_x11_display_get_xdisplay()`
   are declared `extern "C"` in `overlay.rs` to obtain native X11 handles from GDK4 surfaces,
   used to set WM properties (`_NET_WM_WINDOW_TYPE_NOTIFICATION`, `_NET_WM_STATE`) and position
   windows via `XMoveWindow`. The layer overlay aligns to a configurable monitor/corner
   (recomputed each show for hotplug and content-driven height); the "Please Wait" window
   centers across the union of all monitors.

2. **Key grab + input synthesis** -- the key-grab thread `XGrabKey`s the F13–F24 keycodes on
   the root window (for the ignored-modifier combinations), reads `KeyPress`/`KeyRelease` via
   `XNextEvent`, and injects glyphs with `XTestFakeKeyEvent`. F13–F18 resolve through the X
   keymap; F19–F24 are absent from the keymap, so their keycodes are derived from the evdev
   codes (`KEY_F13`=183 … +8 XKB offset → 191–202). There is no keyd IPC anymore.

3. **Non-BMP clipboard delivery** -- the clipboard-owner thread (`clipserve.rs`) owns the
   `CLIPBOARD` selection on its own `Display` and speaks the X11 selection protocol directly:
   `XSetSelectionOwner` to claim, `XConvertSelection` + `XGetWindowProperty` to fetch the prior
   content, and `XChangeProperty` + `XSendEvent(SelectionNotify)` to answer `SelectionRequest`s
   (TARGETS → `[UTF8_STRING, STRING, TARGETS]`; data → the bytes). Ownership release is tied to
   the consumer's observed data request, never a timer (ANTI-PATTERNS #24). The focus classifier
   (`awtfocus.rs`) reads `_NET_ACTIVE_WINDOW`/`XGetClassHint` on the grab `Display`.

## Module responsibilities

| Module | Thread | Responsibility |
|--------|--------|----------------|
| `main.rs` | main | CLI parsing (`type`, `show`, daemon), GTK `Application` setup, thread spawning, 100ms poll (layer change / show request / "Please Wait" flag), panic hook, X error handler install |
| `settings.rs` | main | `config.toml` loading, CLI merge, defaults (see SCHEMA.md) |
| `layers.rs` | main (load) | `layers.toml` parser → navigation ring, `[keys]` physical→logical alias table, per-(layer, logical-key) typing map, and the overlay `LayoutInfo` model. F-keys live only here and at the grab boundary |
| `overlay.rs` | main | GTK4 layer overlay + "Please Wait" window, shared app header (embedded SVG icon `assets/icon.svg` + title), corner-aligned header, font-size fallback, FlowBox legends, content-driven height, dismiss timer |
| `monitor.rs` | main | `MonitorGeometry`, the `Corner` enum (parse, header alignment, per-corner placement), monitor selection (connector/model/index → rightmost fallback), desktop-center |
| `wmprops.rs` | main | X11 window-manager property configuration (EWMH `_NET_WM_*`) via direct Xlib FFI — notification type, above/sticky/all-desktops, focus-less, and window move |
| `xgrab.rs` | key-grab | `XGrabKey` F13–F24, poll-based event loop, layer-ring state machine, debounce/lazy remap scheduling, drives `LayerTyper`, publishes the active layer name and the "Please Wait" flag |
| `xtype.rs` | key-grab + socket server | `LayerTyper` (batch per-layer keycode remap for the grab thread) and `XTyper` (per-glyph remap + LRU for the socket server's manual `type`); both with their own Xlib `Display` |
| `awtfocus.rs` | key-grab | Focus-gated AWT detection: reads `_NET_ACTIVE_WINDOW` (managed top-level) + `XGetClassHint`, matches `WM_CLASS` against the `[non_bmp].clipboard_apps` anchored-glob whitelist. Sampled per keystroke at delivery to decide whether a non-BMP glyph routes to the clipboard |
| `clipserve.rs` | clipboard-owner | Persistent `CLIPBOARD` owner with its own `Display`: on a paste it saves the prior selection, claims ownership, synthesizes Ctrl+V, serves the `SelectionRequest` protocol with **event-driven** release, then restores and keeps serving the saved content until `SelectionClear`. Receives glyphs from the grab thread over an `mpsc` channel |
| `xerror.rs` | process-global | Installs non-fatal `XSetErrorHandler` / `XSetIOErrorHandler` so X protocol errors (e.g. `BadAccess` from `XGrabKey`) log and continue instead of `exit()`-ing |
| `server.rs` | socket server | Unix socket listener; parses `type <char>` / `show`; delegates to `XTyper` or sets the show flag |
| `client.rs` | (separate process) | Connects to the daemon socket, sends `type <char>\n` / `show\n`, exits |
| `socket.rs` | any | Socket path resolution (`$XDG_RUNTIME_DIR`, then `/run/user/*/` scan) |
| `config.rs` | — | `LayoutInfo`/`ButtonGroup`/`ButtonLegend` — the overlay model structs, built by `layers.rs` and read by `overlay.rs` |

## Concurrency model

The daemon runs four concurrent activities, each with its **own** Xlib `Display` connection
(Xlib is not thread-safe across a shared connection; no connection is shared between threads):

1. **GTK main loop** (main thread) -- owns the overlay and "Please Wait" windows; polls a
   `Mutex<String>` (active layer) every 100ms and shows the overlay on change; shows/hides the
   "Please Wait" window on the `please_wait` flag edges; handles the socket-server show flag.
2. **Key-grab thread** -- owns a grab `Display` and a `LayerTyper` `Display`. `XGrabKey`s
   F13–F24, then runs a `poll(2)`-based loop on the X connection fd. Knob keys step the layer
   ring (publishing the layer name) and flag a remap; typeable keys inject the active layer's
   glyph via `LayerTyper` on key **release** (the active grab swallows injection on press).
3. **Socket server** -- owns an `XTyper` `Display`; serves `rologlyphex type`/`show` for manual
   CLI use.
4. **Clipboard-owner thread** -- spawned by the grab thread only when `[non_bmp].clipboard_apps`
   is configured. Owns its own `Display` + a hidden window, and persistently owns/serves the
   `CLIPBOARD` selection (see *Non-BMP clipboard routing*). Idle-blocks on its command channel
   when it owns nothing; polls its X connection while serving.

Shared state (all `Arc`): `Mutex<String>` active-layer name (grab → GTK); `AtomicBool`
`please_wait` (grab → GTK); `AtomicBool` show-request (socket → GTK); `RwLock<HashMap<String,
LayoutInfo>>` overlay model (built once from `LayersConfig`, read by the overlay). Cross-thread
channel: `mpsc::Sender<String>` for glyphs (grab → clipboard-owner).

### Remap scheduling

Only the active layer's ≤10 glyphs are mapped at a time, onto a fixed set of scratch keycodes
(`LayerTyper`), and the whole set is rebuilt in a **single** `XChangeKeyboardMapping` per layer
change — never per keypress, and never mid-spin (a `MappingNotify` storm degrades the whole X
session; see ANTI-PATTERNS #21). Two modes (`remap_mode`):

- **lazy** (default) -- remap on the first keypress in a layer, showing the "Please Wait"
  overlay during the (briefly blocking) keymap rebuild.
- **debounce** -- remap in the idle gap `nav_settle_ms` after the knob settles, no indicator.

### Non-BMP clipboard routing

Java/AWT (notably JetBrains IDEs) truncates non-BMP keysyms via `(int)(keysym & 0xFFFF)`, so
emoji typed through the XTest/keysym path render as wrong CJK glyphs there. AWT's *clipboard*
path is unaffected, so non-BMP glyphs are delivered by paste **only when a Java/AWT window is
focused** — every other glyph, and non-BMP into any non-AWT app, takes the untouched keysym
path.

```
key release (grab thread)
      │  glyph is non-BMP (> U+FFFF)?
      ▼
 awtfocus: focused WM_CLASS ∈ [non_bmp].clipboard_apps ?   ── no ──► keysym path (press)
      │ yes                                                              (terminals, GTK, …)
      ▼  mpsc::Sender<String>
 clipboard-owner thread (clipserve):
   save prior CLIPBOARD → claim → synthesize Ctrl+V → serve SelectionRequests
   → on the consumer's data request (+grace): restore prior content, keep serving
```

Key properties:

- **Focus gate** (`awtfocus.rs`) is the whole reason the earlier global clipboard attempt was
  abandoned (Ctrl+V is literal-insert in terminals, ANTI-PATTERNS #4). The match is an anchored
  `WM_CLASS` glob (`*` the only wildcard) against `res_name` or `res_class`, sampled **per
  keystroke at delivery time** — never cached at a layer boundary, because focus can drift
  during the lazy keymap remap (a TOCTOU window). It is a strong mitigation, not a guarantee:
  the synthetic event and the async paste leave a small residual window where focus could move.
- **Event-driven release** (ANTI-PATTERNS #24): the owner holds the selection until the
  consumer's data request (`UTF8_STRING`/`STRING`, not a `TARGETS` probe) is answered, then a
  grace window for re-polls; a hard cap is only a backstop.
- **Save / restore**: the prior `CLIPBOARD` is fetched before claiming and re-served after the
  paste is consumed, so the user's clipboard is restored rather than clobbered. Caveat: a
  clipboard *manager* (e.g. klipper) may still capture the transient emoji into its history,
  and if the user copies something else mid-delivery it wins (`SelectionClear` drops our state).
- **Non-fatal**: if the owner thread can't start, non-BMP→AWT silently falls back to the keysym
  path. If `[non_bmp].clipboard_apps` is unset, the thread isn't spawned and the path is off.

For the exact `config.toml` / `layers.toml` field definitions, see **SCHEMA.md**.
