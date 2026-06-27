# Plan: Targeted non-BMP input for Java/AWT applications

## Problem

Non-BMP characters (emoji, U+10000+) typed via the XTest/keysym path produce wrong
glyphs in Java/AWT-based applications (notably JetBrains IDEs). See `sdd/ISSUES.md`
entry D. Every other tested application class (terminals, browsers, GTK/Qt) renders
them correctly via the keysym path.

This plan is deliberately **narrow**: it does *not* propose a new global input
mechanism. It proposes routing non-BMP glyphs through the clipboard **only when the
focused window belongs to a Java/AWT application**, leaving the working keysym path
untouched everywhere else. An earlier, broader clipboard attempt was rejected; this
scoping is what makes the approach worth revisiting.

### Root cause

Java's AWT/Swing translates incoming X11 key events through a keysym table and
truncates Unicode keysyms via `(int)(keysym & 0xFFFF)`. The encoding the daemon uses
for non-BMP — `0x01000000 + codepoint` — exceeds 16 bits, so the high bits are
discarded and a wrong (usually CJK) BMP glyph is produced. This is specific to the
**keysym** code path; it is not a general AWT text-handling defect.

## Verified premise

Two tests, receiving side then delivery side, confirm the approach end-to-end.

**Receiving side (2026-06-26)** — tested under the **JetBrains Runtime**
(`idea-IU-233/jbr`, JBR 17), the exact runtime the IDEs use:

- `Toolkit.getSystemClipboard().getData(stringFlavor)` returned `😀🎉✓`
  (`U+1F600 U+1F389 U+2713`) intact — surrogate pairs preserved.
- A **real `Ctrl+V` paste** into a focused `JTextField` produced the same codepoints
  with no truncation, confirmed both programmatically and visually on-screen.

**Delivery side (2026-06-27)** — tested against a *real running JetBrains IDE*
(a RustRover instance editing this very file), with a standalone C probe that mirrors
the salvaged selection-owner code below but applies the #24 fix (event-driven release):

- The probe claimed `CLIPBOARD` with `U+1F600 U+1F389 U+2713`, synthesized `Ctrl+V`,
  and served the `SelectionRequest` protocol. RustRover's JVM polled `TARGETS` ~25 times
  and requested `UTF8_STRING` repeatedly — the exact multi-request, consumer-driven
  behavior the old fixed-timeout release used to lose.
- Releasing ownership **only after the data request was answered, plus a 1.5 s grace**
  survived all of that polling with no premature release. The pasted glyphs landed in the
  editor and, after autosave, were verified **on disk** as `U+1F600 U+1F389 U+2713` —
  intact, no CJK truncation.

**Conclusion**: AWT's *clipboard* path is a separate code route that handles non-BMP
correctly, and the focus-gated selection-owner delivery works against a real IDE consumer
once release is event-driven rather than timed. The receiving side was never the blocker,
and the delivery race (#24) now has a demonstrated fix — what remains is porting it into
the daemon and adding the focus gate.

## Previously attempted

A clipboard-paste path was implemented twice and reverted twice. Both failures are
documented as anti-patterns; the salvageable code (dispatch gate + hardened
`SelectionRequest` serving) is recovered and embedded under "Technical details
preserved" below.

1. **Ctrl+V is literal-insert in terminals** (ANTI-PATTERNS #4). Fatal *only* because
   the old attempt pasted **globally**. A focus gate that never sends Ctrl+V to a
   terminal sidesteps this entirely — this is the crux of the targeted approach.
2. **Ownership-release race** (ANTI-PATTERNS #24). The implementation released
   `CLIPBOARD` ownership on a fixed timeout. X11 paste is asynchronous: the consumer
   requests the selection data *after* it processes Ctrl+V, sometimes after the
   timeout already fired, so the paste intermittently delivered nothing. There is no
   correct fixed timeout. This is the real engineering problem this plan must solve.

## Possible approaches

### 1. Focus-gated clipboard, release after the selection is actually served

Detect that the focused top-level window is Java/AWT (see "Detection" below). Only
then: save the user's current clipboard, claim `CLIPBOARD`, synthesize Ctrl+V, serve
incoming `SelectionRequest` events, and release ownership **only after a request has
actually been answered** (an event-driven signal), not on a blind timer. Restore the
saved clipboard afterward.

**Pros**: Narrow blast radius — terminals/browsers/GTK untouched. Directly kills the
#24 race by tying release to the observed request rather than a guessed delay.
**Cons**: Save/restore is itself racy (restoring before the consumer reads can refire
#24); needs a grace window. Reliable AWT detection required. Clipboard managers that
snapshot history may capture the transient emoji.

### 2. Focus-gated clipboard, retain ownership persistently

As above but never restore — hold ownership until the user's next real copy.

**Pros**: Simplest; eliminates the restore race.
**Cons**: Clobbers the user's clipboard until they copy something else — a visible
papercut. Daemon must keep serving selection requests indefinitely.

### 3. Keep keysym everywhere; surface a graceful fallback for AWT

No clipboard at all. When a non-BMP glyph is requested and an AWT window is focused,
skip typing and notify the user to use the OS emoji picker.

**Pros**: Zero new failure modes; trivial.
**Cons**: Doesn't actually type the glyph — abandons the feature for the one case that
motivated it.

## Recommendation

Pursue **Approach 1**. The verified premise removes the receiving-side risk *and* the
delivery-side risk (the #24 release race now has a probe-confirmed fix), and the focus
gate neutralizes the terminal failure (#4) by construction. Remaining work, in order:

1. **AWT focus detection** — prove a reliable, fast classifier before any clipboard code.
2. **Release timing** — *validated by the delivery probe* (2026-06-27): event-driven
   release (serve until the data request is answered, then a grace window) beat a real
   JVM consumer's repeated polling. Port that serve/release loop into the daemon,
   replacing the salvaged code's timed `deadline`; no further timeout experimentation.
3. **Clipboard save/restore** — add last, with a grace window; fall back to Approach 2's
   retain-ownership behavior if restore proves unreliable.

If step 2 cannot be made reliable, fall back to Approach 2 (retain ownership) rather
than reintroducing a timer. If neither is acceptable, Approach 3 is the floor.

## Technical details preserved

**Detection (focused window is Java/AWT)**: read `_NET_ACTIVE_WINDOW` (or
`XGetInputFocus`, walking up to the top-level), then either `XGetClassHint` (WM_CLASS —
JetBrains windows report `jetbrains-*`; generic AWT reports `java`/`sun-awt-X11-*`) or
`_NET_WM_PID` → `/proc/<pid>/cmdline` and match a JVM. Cache per-window-id to keep the
hot path cheap.

**Dispatch gate**:
```rust
fn type_char(&mut self, ch: char) {
    if (ch as u32) > 0xFFFF {   // non-BMP
        self.type_via_clipboard(&ch.to_string());
        return;
    }
    // ... existing keysym path ...
}
```
In the targeted version this gate is additionally conditioned on AWT focus; otherwise
non-BMP also falls through to the keysym path (correct for non-AWT apps).

**Salvaged delivery code.** The two functions below are the expensive-to-rederive
part — the X11 selection-serving protocol — recovered verbatim from the
twice-shelved implementation and embedded here so it survives in-tree (the original
dangling commits are unreferenced and will be garbage-collected). The code is reusable
**as-is except** for the marked release-timing defect (ANTI-PATTERNS #24).

State the typer needs (interned once in `open()` alongside a 1×1 hidden
`clip_window` created on the root):

```rust
clip_window: xlib::Window,   // XCreateSimpleWindow(dpy, root, 0,0, 1,1, 0,0,0)
clipboard:   xlib::Atom,     // XInternAtom "CLIPBOARD"
utf8_string: xlib::Atom,     // XInternAtom "UTF8_STRING"
targets:     xlib::Atom,     // XInternAtom "TARGETS"
```

`type_via_clipboard()` — claim ownership, synthesize Ctrl+V, serve requests. **The
timed `deadline` / unconditional release at the end is the #24 race — replace the
release with an event-driven trigger (release after the data request is served, then a
grace window) and only then restore the saved clipboard:**

```rust
/// Paste text via clipboard: claim CLIPBOARD, send Ctrl+V, serve the selection request.
fn type_via_clipboard(&self, text: &str) {
    let text_bytes = text.as_bytes();
    unsafe {
        // Drain any stale events on our connection
        while xlib::XPending(self.display) > 0 {
            let mut discard: xlib::XEvent = std::mem::zeroed();
            xlib::XNextEvent(self.display, &mut discard);
        }

        // Claim clipboard ownership
        xlib::XSetSelectionOwner(self.display, self.clipboard, self.clip_window, xlib::CurrentTime);
        xlib::XFlush(self.display);
        if xlib::XGetSelectionOwner(self.display, self.clipboard) != self.clip_window {
            eprintln!("[🐛DEBUG] Failed to claim clipboard ownership");
            return;
        }

        // Send Ctrl+V
        let ctrl_kc = xlib::XKeysymToKeycode(self.display, keysym::XK_Control_L as u64);
        let v_kc = xlib::XKeysymToKeycode(self.display, keysym::XK_v as u64);
        xtest::XTestFakeKeyEvent(self.display, ctrl_kc as u32, xlib::True, 0);
        xtest::XTestFakeKeyEvent(self.display, v_kc as u32, xlib::True, 0);
        xtest::XTestFakeKeyEvent(self.display, v_kc as u32, xlib::False, 0);
        xtest::XTestFakeKeyEvent(self.display, ctrl_kc as u32, xlib::False, 0);
        xlib::XFlush(self.display);

        // Serve SelectionRequest events. Apps send TARGETS first, then UTF8_STRING,
        // so handle multiple requests, not just one.
        // !!! #24: this fixed 1000ms deadline is the race. A slow consumer (JVM warmup)
        // !!! can request the data AFTER the deadline, after ownership is already gone.
        // !!! Replace with: serve until the data (non-TARGETS) request is answered,
        // !!! with no upper deadline that drops ownership prematurely.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        let mut data_served = false;
        while std::time::Instant::now() < deadline {
            let mut event: xlib::XEvent = std::mem::zeroed();
            if xlib::XPending(self.display) > 0 {
                xlib::XNextEvent(self.display, &mut event);
                if event.get_type() == xlib::SelectionRequest {
                    let req = event.selection_request;
                    let is_data = req.target != self.targets;
                    self.handle_selection_request(&req, text_bytes);
                    if is_data { data_served = true; break; }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if !data_served {
            eprintln!("[🐛DEBUG] Clipboard paste: data was not requested within timeout");
        }

        // !!! #24: releasing here on the timed path is the bug. Tie release to
        // !!! `data_served`, hold a grace window, THEN restore the user's clipboard.
        xlib::XSetSelectionOwner(self.display, self.clipboard, 0, xlib::CurrentTime);
        xlib::XFlush(self.display);
    }
}
```

`handle_selection_request()` — the keeper. Answers the multi-target protocol
(TARGETS → list of formats; UTF8_STRING/STRING → the bytes), refuses unsupported
targets, and supports the obsolete `property == None` convention. Reuse verbatim:

```rust
fn handle_selection_request(&self, req: &xlib::XSelectionRequestEvent, text_bytes: &[u8]) {
    // If property is None, the requestor uses an obsolete protocol — use target as property
    let property = if req.property == 0 { req.target } else { req.property };
    unsafe {
        if req.target == self.targets {
            let supported = [self.utf8_string, xlib::XA_STRING, self.targets];
            xlib::XChangeProperty(
                self.display, req.requestor, property,
                xlib::XA_ATOM, 32, xlib::PropModeReplace,
                supported.as_ptr() as *const u8, supported.len() as i32,
            );
        } else if req.target == self.utf8_string || req.target == xlib::XA_STRING {
            xlib::XChangeProperty(
                self.display, req.requestor, property,
                req.target, 8, xlib::PropModeReplace,
                text_bytes.as_ptr(), text_bytes.len() as i32,
            );
        } else {
            // Unsupported target — send refusal (property = None)
            let mut notify = xlib::XSelectionEvent {
                type_: xlib::SelectionNotify, serial: 0, send_event: xlib::True,
                display: self.display, requestor: req.requestor, selection: req.selection,
                target: req.target, property: 0 /* None = refusal */, time: req.time,
            };
            xlib::XSendEvent(self.display, req.requestor, xlib::False, 0,
                &mut notify as *mut _ as *mut xlib::XEvent);
            xlib::XFlush(self.display);
            return;
        }
        // Send SelectionNotify (success)
        let mut notify = xlib::XSelectionEvent {
            type_: xlib::SelectionNotify, serial: 0, send_event: xlib::True,
            display: self.display, requestor: req.requestor, selection: req.selection,
            target: req.target, property, time: req.time,
        };
        xlib::XSendEvent(self.display, req.requestor, xlib::False, 0,
            &mut notify as *mut _ as *mut xlib::XEvent);
        xlib::XFlush(self.display);
    }
}
```

**Clipboard save/restore (not yet implemented)**: before claiming ownership, capture
the user's current `CLIPBOARD` (request UTF8_STRING from the existing owner); restore it
after the grace window. Restoring too early refires #24 against your own paste — gate
the restore on the same served-request signal, not a timer.
