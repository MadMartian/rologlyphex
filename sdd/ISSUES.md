# Known Issues

| ID | Issue | Severity |
|----|-------|----------|
| A | Keycode pool depletion across restarts | Low (fixed with LRU eviction) |
| E | Caps Lock active causes wrong character output (XKB/core mapping mismatch) | Medium |
| B | 100ms polling latency | Low |
| D | Non-BMP characters (emoji) produce wrong output in Java/AWT-based applications | Medium |
| F | /main IPC event fires on layer navigation, not only on keyd reload | Low |
| H | Root-context socket discovery is vulnerable to TOCTOU via /run/user/* scan | Medium |
| I | No XSetErrorHandler — any X protocol error silently exit()s the daemon | Medium |

## A. Keycode pool depletion across restarts

**Severity**: Low (theoretical)

The daemon permanently remaps unused X11 keycodes to character keysyms via `XChangeKeyboardMapping`. These mappings persist in the X server after the daemon exits. On the next daemon start, those keycodes still have keysyms and no longer appear as "free." Over multiple daemon restarts with varying character sets, the free keycode pool shrinks permanently until the X server is restarted (logout/reboot).

With ~50 free keycodes and ~15 characters per session, exhaustion would require roughly 3-4 restarts with entirely different character sets. In practice, the same characters are used across sessions, so previously-mapped keycodes are found via `XKeysymToKeycode` (cache hit at the X server level) and no new keycodes are consumed.

**Mitigation options**:
- Restore remapped keycodes to `NoSymbol` in `XTyper::drop()` (trades startup performance on next run)
- Scan existing mappings at startup and reclaim keycodes that already have our keysyms (Implemented in `XTyper::open()`)
- Accept the limitation and document it

## B. 100ms polling latency

**Severity**: Low (perceptible)

The GTK main thread polls a `Mutex<String>` every 100ms for layout changes (`glib::timeout_add_local`). This introduces up to 100ms latency between a keyd layout change event and the overlay appearing. It also keeps the main loop doing work every 100ms even when idle.

**Mitigation**: Replace polling with an event-driven approach using `glib::MainContext::channel()`, which would deliver layout changes to the GTK thread with zero latency and zero idle overhead.

## D. Non-BMP characters (emoji) produce wrong output in Java/AWT-based applications

**Severity**: Medium (user-visible, affects emoji layouts in Java/AWT-based IDEs and apps)

The XTest/keysym approach encodes non-BMP characters (U+10000+) as keysyms `0x01000000 + codepoint`. Java's AWT/Swing truncates these via `(int)(keysym & 0xFFFF)`, producing wrong CJK glyphs instead of the intended emoji. BMP characters and all other tested applications (terminal emulators, browsers, GTK apps) are unaffected.

**Approaches investigated and rejected**:
- **Clipboard paste + Ctrl+V** (implemented twice, reverted twice): Race conditions between clipboard ownership release and application data request made it unreliable in practice. Also destroys the user's clipboard, and Ctrl+V is intercepted by terminal emulators as a literal control character.
- **Input method CommitText via D-Bus**: Input method contexts require `FocusIn` from the application side — external push-based injection hangs indefinitely or is rejected. Input methods are pull-based by design.
- **XSendEvent ClientMessage**: Most applications ignore synthetic XSendEvent for security reasons.
- **XDG portal clipboard**: Portal support for Java/AWT applications is unverified; likely not available.

**Mitigation options**:
- Use the application's built-in emoji picker (Edit → Emoji & Symbols or OS-level shortcut) for emoji in Java/AWT applications
- Assign only BMP characters to macropad layouts used while running Java/AWT applications
- Revisit if the Java/AWT keysym handling is fixed upstream

## E. Caps Lock active causes wrong character output (XKB/core mapping mismatch)

**Severity**: Medium (user-visible, reproducible with Caps Lock on)

When Caps Lock is active, characters typed via rologlyphex produce wrong output. Example: '∅' appears as 'X'.

**Root cause**: `XChangeKeyboardMapping` modifies the **core** Xlib keyboard mapping for a keycode. Modern X11 applications use **XKB** (X Keyboard Extension), which maintains its own independent keysym table. When Caps Lock is active, XKB applies its modifier rules using its own keysym table — and for the high keycodes rologlyphex uses (near `max_kc`, e.g., 255), XKB may have its own keysym at the Caps Lock modifier level (e.g., 'X'). The application receives XKB's translation rather than the core mapping set by rologlyphex.

**Approaches not yet tried**:
- Update the XKB mapping in addition to the core mapping (e.g., via `XkbChangeTypesOfKey` or by sending `XkbMapNotify`)
- Send XTest events with explicit modifier state that suppresses Caps Lock (lock mask cleared in the event)
- Use `XkbSetMap` to write the keysym into the XKB table alongside the core mapping

**Mitigation**: Turn off Caps Lock before using macropad character layouts.

## F. /main IPC event fires on layer navigation, not only on keyd reload

**Severity**: Low (wasteful, not harmful)

keyd's `IPC_LAYER_LISTEN` protocol sends the active layer name on every layer change, including returning to the main layer via `setlayout(main)`. The daemon treats every `/main` event as a reload signal: it attempts a config reparse (throttled by the 200ms shared debounce) and sets the XTyper `reload_flag` (triggering `XTyper::rescan()` before the next `type_char()` call). On reload, this is correct behavior. On normal navigation back to main, it is wasteful — a disk read and an X-server round trip happen unnecessarily.

**Mitigation**: Distinguish reload events from navigation events — keyd may expose a separate `IPC_RELOAD` event type. Until then, the spurious reparse on navigation-to-main is a disk read plus an X server round trip; it is not user-visible because the GTK poll's `config_reloaded` flag only triggers a show when the layout actually changed.

## H. Root-context socket discovery is vulnerable to TOCTOU via /run/user/* scan

**Severity**: Medium

When `rologlyphex type` is invoked as root (no `XDG_RUNTIME_DIR`), `socket.rs:65-72` scans `/run/user/*/rologlyphex.sock` and connects to the first matching path. The seat0 check (`socket.rs:44`) runs separately, but path discovery uses `exists()` which is a TOCTOU race — a local unprivileged user can plant a socket or symlink at a matching path between the check and the connect. A malicious socket could cause glyph mis-delivery (wrong characters typed) or act as a DoS against root-invoked type commands. Root-to-user privilege boundary is the bounded threat; this does not grant the attacker arbitrary root execution.

**Mitigation**: Derive the socket path directly from the validated seat0 UID (obtained via the D-Bus login1 query) rather than scanning the filesystem. Then `lstat` the resolved path and verify `S_ISSOCK` and UID ownership before connecting.

## I. No XSetErrorHandler — X protocol errors silently exit() the daemon

**Severity**: Medium

The daemon makes no call to `XSetErrorHandler` or `XSetIOErrorHandler`. If the X server sends an error response to any Xlib call (e.g., `BadWindow` on a destroyed overlay window, `BadAccess` on a keycode already mapped by another client), Xlib's default error handler calls `exit()`. This bypasses Rust's panic hook, produces no log entry, and leaves the daemon socket in place — subsequent `rologlyphex type` calls will connect but get no response until the stale socket times out.

**Mitigation**: Install a custom `XSetErrorHandler` that logs the error details and returns (non-fatal for recoverable errors), and an `XSetIOErrorHandler` that performs a clean shutdown.

