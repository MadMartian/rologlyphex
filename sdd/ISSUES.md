# Known Issues

| ID | Issue | Severity |
|----|-------|----------|
| A | Keycode pool depletion across restarts | Low |
| B | 100ms polling latency | Low |
| C | Double config re-parse on keyd reload | Low |
| D | Non-BMP characters (emoji) produce wrong output in JetBrains IDEs | Medium |

## A. Keycode pool depletion across restarts

**Severity**: Low (theoretical)

The daemon permanently remaps unused X11 keycodes to character keysyms via `XChangeKeyboardMapping`. These mappings persist in the X server after the daemon exits. On the next daemon start, those keycodes still have keysyms and no longer appear as "free." Over multiple daemon restarts with varying character sets, the free keycode pool shrinks permanently until the X server is restarted (logout/reboot).

With ~50 free keycodes and ~15 characters per session, exhaustion would require roughly 3-4 restarts with entirely different character sets. In practice, the same characters are used across sessions, so previously-mapped keycodes are found via `XKeysymToKeycode` (cache hit at the X server level) and no new keycodes are consumed.

**Mitigation options**:
- Restore remapped keycodes to `NoSymbol` in `XTyper::drop()` (trades startup performance on next run)
- Scan existing mappings at startup and reclaim keycodes that already have our keysyms
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

## D. Non-BMP characters (emoji) produce wrong output in JetBrains IDEs

**Severity**: Medium (user-visible, affects emoji layouts in JetBrains IDEs)

The XTest/keysym approach encodes non-BMP characters (U+10000+) as keysyms `0x01000000 + codepoint`. Java's AWT/Swing in JetBrains IDEs truncates these via `(int)(keysym & 0xFFFF)`, producing wrong CJK glyphs instead of the intended emoji. BMP characters and all other tested applications (Konsole, Firefox, GTK apps) are unaffected.

**Approaches investigated and rejected**:
- **Clipboard paste + Ctrl+V** (implemented twice, reverted twice): Race conditions between clipboard ownership release and application data request made it unreliable in practice. Also destroys the user's clipboard, and Ctrl+V is intercepted by terminal emulators as a literal control character.
- **IBus CommitText via D-Bus**: IBus contexts require `FocusIn` from the application side — external push-based injection hangs indefinitely or is rejected. IBus is pull-based by design.
- **XSendEvent ClientMessage**: Most applications ignore synthetic XSendEvent for security reasons.
- **XDG portal clipboard**: Portal support unverified for JetBrains IDEs; likely not available.

**Mitigation options**:
- Use the IDE's built-in emoji picker (Edit → Emoji & Symbols or OS-level shortcut) for emoji in JetBrains IDEs
- Assign only BMP characters to macropad layouts used while coding in JetBrains IDEs
- Revisit if JetBrains fixes AWT keysym handling upstream
