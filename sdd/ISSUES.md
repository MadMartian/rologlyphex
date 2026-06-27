# Known Issues

| ID | Issue | Severity |
|----|-------|----------|
| A | Keycode pool depletion across restarts | Low (fixed with LRU eviction) |
| E | Caps Lock active causes wrong character output (XKB/core mapping mismatch) | Medium |
| B | 100ms polling latency | Low |
| D | Non-BMP characters (emoji) produce wrong output in Java/AWT-based applications | Medium |
| H | Root-context socket discovery is vulnerable to TOCTOU via /run/user/* scan | Medium |
| I | Ambiguous-width Unicode glyphs (e.g. ℃) misrender cursor position in terminal emulators | Closed |

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

The GTK main thread polls a `Mutex<String>` every 100ms for layout changes (`glib::timeout_add_local`). This introduces up to 100ms latency between the grab thread publishing a layer change (`Mutex<String>`) and the overlay appearing. It also keeps the main loop doing work every 100ms even when idle.

**Mitigation**: Replace polling with an event-driven approach using `glib::MainContext::channel()`, which would deliver layout changes to the GTK thread with zero latency and zero idle overhead.

## D. Non-BMP characters (emoji) produce wrong output in Java/AWT-based applications

**Severity**: Medium (user-visible, affects emoji layouts in Java/AWT-based IDEs and apps)

The XTest/keysym approach encodes non-BMP characters (U+10000+) as keysyms `0x01000000 + codepoint`. Java's AWT/Swing truncates these via `(int)(keysym & 0xFFFF)`, producing wrong CJK glyphs instead of the intended emoji. BMP characters and all other tested applications (terminal emulators, browsers, GTK apps) are unaffected.

**Approaches investigated and rejected**:
- **Clipboard paste + Ctrl+V** (implemented twice, reverted twice): Failed for two independent reasons. (1) **Ownership-release race**: the implementation claimed `CLIPBOARD`, synthesized Ctrl+V, served the incoming `SelectionRequest`, then released ownership (`XSetSelectionOwner(.., 0)`) on a short timeout. X11 paste is asynchronous — the consumer requests the selection data *after* it processes the synthetic Ctrl+V, which can arrive after the timeout has already fired and dropped ownership, so the paste intermittently delivered nothing. (2) **Terminal Ctrl+V**: terminal emulators treat Ctrl+V as literal-insert, not paste (see ANTI-PATTERNS #4). It also clobbers the user's clipboard. The salvageable parts of the shelved implementation (dispatch gate + hardened `SelectionRequest` serving) are embedded in `sdd/PLAN.non-BMP.md`.
- **Input method CommitText via D-Bus**: Input method contexts require `FocusIn` from the application side — external push-based injection hangs indefinitely or is rejected. Input methods are pull-based by design.
- **XSendEvent ClientMessage**: Most applications ignore synthetic XSendEvent for security reasons.
- **XDG portal clipboard**: Portal support for Java/AWT applications is unverified (never tested); likely not available.

**Verified**: The truncation is specific to AWT's *keysym* decode path. AWT accepts a non-BMP **paste** correctly (a separate code path delivering UTF-8 selection bytes) — confirmed under the JetBrains Runtime: `😀🎉✓` pasted into a Swing field intact, surrogate pairs preserved. A targeted, focus-gated clipboard approach is therefore tracked in `sdd/PLAN.non-BMP.md`; the open problem is delivery (the #24 ownership-release race), not AWT.

**Mitigation options**:
- Use the application's built-in emoji picker (Edit → Emoji & Symbols or OS-level shortcut) for emoji in Java/AWT applications
- Assign only BMP characters to macropad layouts used while running Java/AWT applications
- Save and restore the user's prior clipboard contents around a paste to avoid clobbering it (itself racy — the restore can land before the consumer reads the injected value)
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

## H. Root-context socket discovery is vulnerable to TOCTOU via /run/user/* scan

**Severity**: Medium

When `rologlyphex type` is invoked as root (no `XDG_RUNTIME_DIR`), `socket.rs:48-56` scans `/run/user/*/rologlyphex.sock` and connects to the first matching path. Path discovery uses `exists()`, which is a TOCTOU race — a local unprivileged user can plant a socket or symlink at a matching path between the check and the connect. A malicious socket could cause glyph mis-delivery (wrong characters typed) or act as a DoS against root-invoked type commands. Root-to-user privilege boundary is the bounded threat; this does not grant the attacker arbitrary root execution.

**Mitigation**: `lstat` the resolved path and verify `S_ISSOCK` and UID ownership before connecting. Better still, don't invoke the client as root at all — the daemon is a user-session client, so the normal `$XDG_RUNTIME_DIR` path applies and the scan is never reached. (The earlier D-Bus seat0 lookup was removed with keyd retirement.)

## I. Ambiguous-width Unicode glyphs (e.g. ℃) misrender cursor position in terminal emulators

**Severity**: Closed (inherent to terminal/font `wcwidth` handling — confirmed independent of rologlyphex)

Certain Unicode characters — e.g. ℃ (U+2103, DEGREE CELSIUS), mapped on a layer key — fall into Unicode's "Ambiguous" East Asian Width class. A terminal's `wcwidth()` reserves one column for them, but many fonts draw the glyph wider than that column, so the terminal's cursor-position bookkeeping and the drawn glyph disagree. Symptom: the glyph appears half-rendered until the next keystroke redraws it, and the cursor visibly drifts from its real column — easy to mistype nearby text as a result.

Confirmed independent of rologlyphex: the identical glitch reproduces from a plain clipboard paste of ℃, with the daemon entirely out of the loop. The daemon's XTest injection sends one correct `KeyPress`/`KeyRelease` for the character's keysym regardless of how a downstream terminal chooses to lay it out.

**What walls every exit**: the mismatch is between a terminal emulator's column-width table and the width its font actually draws — both live entirely outside this project, and vary per terminal/font combination. No keysym choice, injection timing, or XTest parameter available to rologlyphex affects how a downstream client renders a character it already received correctly.

**Mitigation options**:
- None available at the rologlyphex layer — this is a terminal/font rendering property, not an input-injection defect.
- If it recurs often, avoid ambiguous-width glyphs on frequently-used macropad keys, or configure the affected terminal/font to treat ambiguous-width characters as narrow (terminal-specific, outside this project).
