# Known Issues

| ID | Issue | Severity |
|----|-------|----------|
| A | Keycode pool depletion across restarts | Low (fixed with LRU eviction) |
| E | Caps Lock active causes wrong character output (XKB/core mapping mismatch) | Medium |
| B | 100ms polling latency | Low |
| C | Double config re-parse on keyd reload | Low |
| D | Non-BMP characters (emoji) produce wrong output in Java/AWT-based applications | Medium |

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

## C. Double config re-parse on keyd reload

**Severity**: Low (wasteful, not harmful)

When `keyd reload` is run, two things happen nearly simultaneously:
1. The inotify watcher detects the config file change and triggers `reparse_config()`
2. keyd sends a `/main` layout event via IPC, which also triggers `reparse_config()`

The inotify watcher has a 200ms debounce, but the IPC `/main` handler has none and typically fires first. The result is two sequential config parses for one reload event.

**Mitigation**: Add a shared debounce mechanism (e.g., a timestamp of last reparse) checked by both the inotify and IPC paths.

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

