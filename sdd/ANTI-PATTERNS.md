# Anti-Patterns

Coding, functional, and behavioural anti-patterns encountered during development that consumed significant effort through trial-and-error before resolution.

| # | Anti-pattern |
|---|-------------|
| 1 | GTK4 layout timing: measuring before layout |
| 2 | Dynamic window positioning: chasing the right edge |
| 3 | `set_max_width_chars` defeating `measure()` |
| 4 | Clipboard paste Ctrl+V in terminals |
| 5 | `rfind(')')` with nested parentheses |
| 6 | `#[macro_export]` macro visibility across modules |
| 7 | X11 format-32 properties expect `c_long`, not `u32` |
| 8 | `XChangeProperty` alone doesn't update already-mapped windows |
| 9 | `XSync` is not sufficient before `XTestFakeKeyEvent` after `XChangeKeyboardMapping` |
| 10 | idle_add_local_once race on first window map causes focus steal |
| 11 | Pre-filtering CLI arguments |
| 12 | `SupplementaryGroups` in a systemd user service fails with "Operation not permitted" |
| 13 | Low X11 keycodes (near min_kc) silently swallow XTest key events |
| 14 | Multimedia keysyms falsely reclaimed as rologlyphex emoji mappings |
| 15 | GTK4 single-instance mechanism causes silent exit code 0 when another instance is running |
| 16 | GTK4 FlowBox with `halign=Center` ignores allocated width; `set_default_size` ignored on realized windows |
| 17 | keyd hard-limits `command()` calls to 64 per config file — excess bindings silently dropped |
| 18 | Full-keyboard virtual device consumes nearly all X11 keycodes, leaving only ~10 free for XTest remapping |
| 19 | EEPROM chord modifiers (Ctrl+Alt+Shift) bleed through keyd virtual keyboard, corrupting XCompose sequences |
| 20 | Watching the keyd config file with inotify to detect reloads |
| 21 | `XChangeKeyboardMapping` per keycode storms `MappingNotify` and brings the whole desktop UI to its knees |
| 22 | An `XGrabKey` active grab swallows `XTest` injection done on key press |
| 23 | Signaling a UI progress indicator *during* a blocking keymap remap never renders it |
| 24 | Releasing X11 selection ownership on a timeout races the asynchronous paste consumer |

## 24. Releasing X11 selection ownership on a timeout races the asynchronous paste consumer

**Symptom**: A clipboard-paste injection path (claim `CLIPBOARD`, synthesize Ctrl+V, serve the `SelectionRequest`, then release ownership) delivered the pasted text only intermittently. The same code path worked in some applications and silently dropped the paste in others, with no error — the target field simply stayed empty.

**What was tried**:
- Serving the `SelectionRequest` in a loop with a timeout, then calling `XSetSelectionOwner(display, CLIPBOARD, None, CurrentTime)` to relinquish ownership so the user's real clipboard wasn't permanently hijacked.
- Hardening the serve loop (multi-request TARGETS→UTF8_STRING protocol, stale-event draining, obsolete property=None protocol) — improved robustness of *serving* a request but did not fix the *intermittent no-paste*.

**Root cause**: X11 paste is asynchronous and consumer-driven. Synthesizing Ctrl+V only *signals* the focused application to paste; the application then asks the selection owner for the data on its own schedule — often several milliseconds later, and later still for heavyweight toolkits (JVM warmup, event-queue depth). If the owner releases the selection (or the serve loop times out) before that request arrives, the request finds no owner — or finds ownership already transferred back — and the paste yields nothing. There is no synchronous acknowledgement that the consumer has finished reading; a fixed timeout is a guess, and the race is invisible because nothing errors.

**Resolution**: Abandoned the timeout-then-release design for non-BMP input (the approach had a second, independent blocker in terminals — see #4). No correct fixed timeout exists; the owner cannot know the consumer is done without an explicit protocol the consumer doesn't provide. Shelved code embedded in `sdd/PLAN.non-BMP.md`.

**Lesson**: When you own an X11 selection to feed a synthetic paste, you cannot safely release ownership on a timer — the consumer reads the data asynchronously, after it processes the paste keystroke, with no signal back to you when it's finished. Either retain ownership indefinitely (and accept clobbering the user's clipboard) or don't use the selection mechanism for push-style injection at all. A "paste then release after N ms" design is a race with no correct N.

## 22. An `XGrabKey` active grab swallows `XTest` injection done on key press

**Symptom**: After grabbing F13–F24 with `XGrabKey` and injecting the replacement glyph via `XTestFakeKeyEvent` from the `KeyPress` handler, typing was unreliable: often only the first press in a layer emitted, then nothing on subsequent presses until the layer changed — yet the debug log showed every press being handled. The failure was inconsistent and timing-dependent.

**What was tried**:
- Injecting the scratch keycode via `XTestFakeKeyEvent` immediately in the `KeyPress` event handler.
- Suspecting keycode-pool exhaustion / MappingNotify timing (real, but not the cause here).

**Root cause**: An `XGrabKey` passive grab becomes an **active keyboard grab** when the key is pressed, lasting until the key is released. During the active grab, key events — including events synthesized by `XTest` — are routed to the grabbing client (with `owner_events=False`, the grab window), **not** the focused application. So the injected glyph was delivered back to us and dropped. The inconsistency was tap timing: if the physical key was released before the event loop reached the injection (active grab already ended), it landed; otherwise it was swallowed.

**Resolution**: Inject on **`KeyRelease`**, not `KeyPress`. By the time the key is released the active grab has ended, so the synthetic keystroke reaches the focused window. Navigation keys (which do no injection) stay on press for responsiveness.

**Lesson**: When you `XGrabKey` a key and want to inject a replacement with `XTest`, do it on key **release**, not press. The active grab held while the key is down routes synthetic key events to the grabbing client, not the focused window, so press-time injection is swallowed.

## 23. Signaling a UI progress indicator *during* a blocking keymap remap never renders it

**Symptom**: A "Please Wait" overlay, shown by setting a shared `AtomicBool` that the GTK 100ms poll watches, never appeared — even after holding the flag `true` for 350ms (more than three poll intervals). The grab thread logged setting the flag; the GTK poll never logged showing the window.

**What was tried**:
- Setting the flag `true`, running the remap, then holding the flag a minimum time before clearing it, trusting the 100ms poll to sample it.
- Adding a `set_size_request` to the window and tracing the flag through every hop (the Arc sharing and the poll were both correct).

**Root cause**: The remap's single `XChangeKeyboardMapping` broadcasts a `MappingNotify`, and GDK — GTK's own X client — rebuilds its keymap in response, which **blocks the GTK main loop** for the duration on a keycode-heavy system. That stall overlaps precisely the window in which the flag is `true`, so the GTK poll cannot run to display the indicator until *after* the remap, by which point the flag is already cleared.

**Resolution**: Raise the flag and `sleep` a lead time (longer than the poll interval, ~220ms) **before** the remap, so the GTK poll displays the indicator while the main loop is still free. Then run the remap — which blocks GTK, but the indicator is already mapped and visible.

**Lesson**: A progress indicator for a blocking operation must be made visible **before** the operation starts, not signaled to appear during it. If the operation stalls the UI thread (here, via `MappingNotify`-triggered keymap rebuilds in every X client), the UI cannot render the indicator until the operation has already finished.

## 21. `XChangeKeyboardMapping` per keycode storms `MappingNotify` and brings the whole desktop UI to its knees

**Symptom**: After retiring keyd and routing **all** macropad typing through rologlyphex's XTest keycode-remap path, navigating layers with the knob made the **entire desktop** sluggish — mouse scrolling stuttered, window interactions lagged, the UI "crawled." It was not confined to the daemon or the focused app; the whole X session degraded. Killing the daemon restored performance instantly.

**What was tried**:
- Per-keypress remapping (the original `XTyper` path): with the secondary keyboard leaving ~0 free keycodes (#18), every press did an `XChangeKeyboardMapping`. Typing thrashed and was unreliable (#9), but the system-wide impact was masked because typing was relatively infrequent and interleaved with idle time.
- A per-layer batch remap — but implemented as one `XChangeKeyboardMapping` call **per scratch keycode** (10 calls), and executed on **every knob step**. The debug log of one short test showed 44 knob steps → ~**440** `XChangeKeyboardMapping` calls in rapid succession.

**Root cause**: `XChangeKeyboardMapping` is not a local, per-keycode mutation — the X server broadcasts a `MappingNotify` event to **every connected client** on each call, and each client (browser, plasmashell, kwin, every GUI app) must stop and reprocess the keymap (`XRefreshKeyboardMapping`). Issuing hundreds of these in a burst (10 per keycode × dozens of knob steps) creates a server-wide event storm: every client is repeatedly interrupted to rebuild its keymap, starving the whole session of responsiveness. The cost scales with the number of connected X clients, not with the daemon's own work — which is why it surfaced as global UI degradation, not a local slowdown.

**Resolution**:
1. **Never remap on navigation.** The grab thread only publishes the active layer name on each knob step (instant overlay tracking); the keymap remap is deferred and happens lazily on the *first keypress* in a layer. Spinning the knob now causes **zero** keymap changes.
2. **One call, not N.** `LayerTyper::set_layer` issues a **single** `XChangeKeyboardMapping` over the contiguous keycode range spanning the scratch keycodes (reading the range first to preserve every non-scratch keycode), producing **one** `MappingNotify` instead of one per key.

Net: idle navigation = 0 remaps; worst case = 1 `MappingNotify` the first time you type in a freshly-entered layer (down from ~440 across a navigation session).

**Lesson**: `XChangeKeyboardMapping` is a **global broadcast**, not a cheap local change — every call interrupts every X client to rebuild its keymap. Treat it as expensive and rare: (a) batch all changes into a single call over a contiguous keycode range (read the range first so neighbours are preserved), and (b) never trigger it on high-frequency or idle events (navigation, polling, hover) — only when output is actually about to happen. On a host where a keyboard-management daemon (a keyboard-management daemon + a full keyboard) already churns the keycode pool, a `MappingNotify` storm can take the entire desktop down, not just the daemon.

## 16. GTK4 FlowBox with `halign=Center` ignores allocated width; `set_default_size` ignored on realized windows

**Symptom**: Switching from a fixed horizontal `Box` to a `FlowBox` for the button legend caused two separate failures, each masking the other:

1. Emoji layouts (wider cells) overflowed the right edge of the overlay window despite the window having a fixed width.
2. After fixing overflow by adjusting `min_children_per_line`, the window gained large amounts of empty vertical space below the content, extending off the bottom of the screen.

**What was tried**:
- `set_min_children_per_line(5)` — stopped overflow for math symbol layouts (5 × ~68px = 340px < 600px) but not emoji layouts (5 × ~100px = 500px, still fits), yet then set to a value where emoji DID overflow
- `set_homogeneous(false)` — caused each row to contain exactly 1 item because FlowBox measures natural width as 1 cell
- Calling `set_default_size(window_width, natural_h)` in `show_layout()` to dynamically resize the window height — had no effect at all

**Root cause**: Two independent GTK4 behaviors combined:

1. **`FlowBox` with `halign=Center` ignores allocated width.** A `FlowBox` with `halign=Center` calculates row breaks using its *natural* width, not the width allocated to it by its parent. Natural width = `min_children_per_line × max_cell_width`. Emoji cells are wider than math cells, so with `halign=Center` the FlowBox could overflow the container even though the container was correctly constrained to 600px.

2. **`set_default_size` is ignored on already-realized windows.** `WidgetExt::realize()` is called explicitly in `Overlay::new()` (necessary for setting X11 properties before first map). After `realize()`, the window is already realized and `set_default_size()` calls from `show_layout()` are silently ignored — they only take effect before realize.

**Resolution**:
- Set `set_halign(Align::Fill)` on both the `content_box` and the `legend_box` (FlowBox). `Align::Fill` forces the widget to use its *allocated* width for layout decisions, so FlowBox wraps at the actual container boundary regardless of cell size.
- Replace `set_default_size(w, natural_h)` with `set_size_request(w, natural_h)` in `show_layout()`. `set_size_request` works on already-realized windows; `set_default_size` does not.
- Compute `natural_h` via `content_box.measure(Orientation::Vertical, window_width)` — this performs correct height-for-width computation at the constrained width, returning the actual height needed for the wrapped content.

**Lesson**: `FlowBox` with `halign=Center` uses natural width for wrapping — this is almost never what you want for a width-constrained container. Use `halign=Fill` so wrapping is driven by allocated width. For dynamic window height on an already-realized GTK4 window, use `set_size_request()`, not `set_default_size()`. Measure content height with `widget.measure(Vertical, constrained_width)` after setting content, before calling `set_size_request`.

## 1. GTK4 layout timing: measuring before layout

**Symptom**: `XGetGeometry`, `widget.width()`, and `widget.measure()` returned stale or default values when called from `idle_add_local_once` after showing a window.

**What was tried**:
- `idle_add_local_once` after `set_visible(true)` — fires before GTK has laid out content
- `connect_map` + nested idle callback — still too early
- `timeout_add_local_once(50ms)` — sporadic, timing-dependent

**Root cause**: GTK4 defers layout until the main loop processes pending events. There is no reliable single callback that fires "after layout is complete" for a newly-shown window.

**Resolution**: Abandoned dynamic sizing. Used a fixed-size window with deterministic positioning calculated from monitor geometry and window dimensions (both known at construction time). Font-size reduction uses `Label::measure()` which works correctly after the label's text and CSS class are set, even before the window is shown.

**Lesson**: Do not attempt to measure rendered widget geometry in GTK4 during show/realize. If you need dimensions, use known constants or `widget.measure()` with explicit constraints. GTK4 removed `connect_size_allocate` (a GTK3 API), so there is no post-layout callback.

## 2. Dynamic window positioning: chasing the right edge

**Symptom**: Overlay window appeared at inconsistent horizontal positions, sometimes off-screen, sometimes truncated, sometimes flickering.

**What was tried**:
- Querying window width after show via `XGetGeometry`
- Querying via `widget.width()` in idle callbacks
- Computing position from natural content width

**Root cause**: All approaches depended on anti-pattern #1 — measuring geometry before GTK4 had completed layout.

**Resolution**: Width-configurable window (default 600, height calculated automatically from content). Position calculated as `monitor_x + monitor_width - window_width - margin` using values known at construction time.

**Lesson**: For overlay/notification windows, prefer fixed dimensions over content-adaptive sizing. The layout timing problem is a deep GTK4 constraint, not a bug to be worked around.

## 3. `set_max_width_chars` defeating `measure()`

**Symptom**: Font-size reduction logic had "no effect" — the smaller font was never triggered regardless of title length.

**What was tried**:
- Adding the font reduction code (correct logic)
- Debugging with print statements showing natural width was always tiny

**Root cause**: `Label::set_max_width_chars(1)` was set on the title label. This constrained the label's reported natural width to approximately one character width, so `measure(Orientation::Horizontal, -1)` always returned a small value that never exceeded `available_width`.

**Resolution**: Removed `set_max_width_chars(1)`. The label's natural width now reflects its actual text content, and `measure()` returns meaningful values for the overflow check.

**Lesson**: `set_max_width_chars` constrains `measure()` results, not just rendering. Do not set it on labels whose natural width you intend to query.

## 4. Clipboard paste Ctrl+V in terminals

**Symptom**: Emoji characters typed via clipboard paste appeared as `^V^V^V...` in terminal emulators.

**What was tried**:
- Implementing full X11 clipboard ownership (claim CLIPBOARD, send Ctrl+V via XTest, serve SelectionRequest events)
- Hardening for multi-request protocol (TARGETS then UTF8_STRING)
- Adding timeout handling, stale event draining, refusal for unsupported targets
- Supporting obsolete protocol (property=None)

**Root cause**: Terminal emulators interpret Ctrl+V as "insert next character literally" (a readline/VT convention), not as "paste from clipboard." This is fundamental to how terminals work and cannot be detected or worked around at the X11 level.

**Resolution**: Reverted the non-BMP clipboard gate. All characters now use the keysym remap path. Clipboard code was eventually removed entirely from the working tree; the salvageable code is embedded in `sdd/PLAN.non-BMP.md` (see `sdd/ISSUES.md` entry D for the full investigation). The terminal problem is one of two independent failure modes — the other is the ownership-release race in #24.

**Lesson**: Ctrl+V is not a universal paste operation. Before building a clipboard-based input mechanism, verify it works in ALL target application types (GUI editors, terminals, IDEs, browsers). Test terminals early — they are the most likely to break.

## 5. `rfind(')')` with nested parentheses

**Symptom**: `macro2(120, 80, macro(Hello space World))` extracted `"Hello space World)"` with a trailing `)`.

**What was tried**:
- Using `rfind(')')` to find the closing paren of `macro()` — this found the outermost `)` of the entire `macro2(...)` expression
- Checking for `macro2` prefix to avoid the `macro()` handler — but `action.find("macro(")` matched the inner `macro(` before the `macro2()` handler ran

**Root cause**: Two issues compounded: (a) `rfind(')')` is greedy and finds the last `)` in the entire string, not the matching one; (b) the `macro()` check ran before the `macro2()` check, so `macro2(... macro(...))` was parsed by the wrong handler.

**Resolution**: Reordered checks to try `macro2()` before `macro()`. Since `macro2(` does not contain the substring `macro(` (it's `macro2(`), and the inner `macro(` is only reached via recursive `extract_display_char()` on the third argument, the nesting resolves correctly.

**Lesson**: When parsing nested expressions with `rfind`/`find`, check the outermost wrapper first. `rfind(')')` is never correct for matching parentheses in expressions that may contain nested parens — it always finds the wrong one.

## 6. `#[macro_export]` macro visibility across modules

**Symptom**: 17 compilation errors — `cannot find macro 'debug_log' in this scope` in every module.

**What was tried**:
- Defining `#[macro_export] macro_rules! debug_log` in `main.rs`
- Expecting it to be automatically available in submodules (as it would be without `#[macro_export]` in the same file)

**Root cause**: `#[macro_export]` hoists the macro to the crate root namespace, but submodules must explicitly import it with `use crate::debug_log;`. Without `#[macro_export]`, `macro_rules!` macros are textually scoped and only available after their definition point in the same file.

**Resolution**: Added `use crate::debug_log;` to each module that uses the macro.

**Lesson**: In Rust, `#[macro_export]` macros require `use crate::macro_name;` in other modules within the same crate. This is different from how `macro_rules!` works without `#[macro_export]` (textual scoping within the defining file).

## 7. X11 format-32 properties expect `c_long`, not `u32`

**Symptom**: `_NET_WM_DESKTOP` set to `0xFFFFFFFF` (all desktops) had no effect or caused erratic behavior.

**What was tried**:
- `let all_desktops: [u32; 1] = [0xFFFFFFFF];` passed to `XChangeProperty` with format 32

**Root cause**: X11's "format 32" properties use `c_long` (8 bytes on x86_64 Linux), not 32-bit integers. The "32" refers to the logical element size in the X protocol, but the C API uses `long` for historical reasons. Passing a `u32` pointer causes `XChangeProperty` to read 8 bytes from a 4-byte allocation, pulling garbage from adjacent stack memory.

**Resolution**: Changed to `[std::ffi::c_long; 1] = [0xFFFFFFFFu32 as std::ffi::c_long]`.

**Lesson**: In Xlib FFI, format 32 = `c_long`, format 16 = `c_short`, format 8 = `c_char`. Never use fixed-width integer types (`u32`, `u16`, `u8`) for `XChangeProperty` data — they are wrong on 64-bit systems.

## 8. `XChangeProperty` alone doesn't update already-mapped windows

**Symptom**: Setting `_NET_WM_STATE` and `_NET_WM_DESKTOP` via `XChangeProperty` had no visible effect — the window stayed on one desktop and didn't become sticky.

**What was tried**:
- Setting properties via `XChangeProperty` in `configure_x11_properties()` (called after the window is shown)

**Root cause**: The EWMH specification requires that changes to `_NET_WM_STATE` and `_NET_WM_DESKTOP` on already-mapped (visible) windows be communicated via `ClientMessage` events sent to the root window. The window manager monitors these events. `XChangeProperty` only sets the property on the window's property list — KWin doesn't re-read properties after mapping, it listens for ClientMessages.

**Resolution**: After `XChangeProperty`, send `XSendEvent` with `ClientMessage` to the root window for each property change: `_NET_WM_DESKTOP` (with `0xFFFFFFFF`), and `_NET_WM_STATE` (with `_NET_WM_STATE_ADD` action for ABOVE, STICKY, SKIP_TASKBAR, SKIP_PAGER).

**Lesson**: For EWMH window properties on already-visible windows, `XChangeProperty` is necessary but not sufficient. Always follow it with a `ClientMessage` to the root window. The `XChangeProperty` call sets the initial value (used if the WM reads it before mapping), and the `ClientMessage` notifies the WM of runtime changes.

## 9. XSync is not sufficient before XTestFakeKeyEvent after XChangeKeyboardMapping

**Symptom**: Characters typed into apps started via `pkexec` (elevated privileges) appeared as wrong or garbled glyphs on first use of a new character.

**What was tried**:
- `XSync(display, False)` after `XChangeKeyboardMapping` before `XTestFakeKeyEvent` — ensures the X server has processed the mapping change, but does not wait for receiving clients to process their `MappingNotify` event

**Root cause**: `XChangeKeyboardMapping` causes the X server to broadcast a `MappingNotify` event to all connected clients. Each client must call `XRefreshKeyboardMapping` (or equivalent) in response to update its internal keymap. `XSync` only confirms the server has processed our request — it gives no guarantee that any other client has yet received or acted on the `MappingNotify`. If `XTestFakeKeyEvent` fires before the focused window refreshes its keymap, the window translates the keycode using its stale mapping and produces the wrong character. Apps started via `pkexec` are especially susceptible due to scheduling differences (separate process, possible elevated-priority overhead) that increase the time between `MappingNotify` delivery and keymap refresh.

**Resolution**: Added a 15ms `std::thread::sleep` after `XSync` in `remap_and_type()`, giving clients time to process `MappingNotify` before the key event arrives. This delay only applies on first use of each new character; subsequent presses use the cached keycode and incur no delay.

**Lesson**: `XSync` after `XChangeKeyboardMapping` is necessary but not sufficient. Other X11 clients need time to process the resulting `MappingNotify` before keycode-to-keysym translation will be correct. A short sleep (10–20ms) is the standard pragmatic workaround; the theoretically correct solution (waiting for all clients to ack) is not possible in X11.

## 10. idle_add_local_once race on first window map causes focus steal

**Symptom**: The overlay stole focus from the active window on its very first appearance after daemon start, despite GTK-level focusable=false hints. Subsequent shows were fine.

**What was tried**: `connect_realize` + `idle_add_local_once` to apply X11 properties after the window surfaces.

**Root cause**: The WM processes `MapRequest` before `idle_add` fires. On first map the WM sees a window without `_NET_WM_USER_TIME=0` or `WM_HINTS{input=False}` and grants focus. Subsequent shows work because the properties are already set and persistent.

**Resolution**: Call `WidgetExt::realize()` explicitly in `new()` before first show, then call `configure_x11_properties()` immediately and synchronously. Also set `_NET_WM_USER_TIME=0` and `WM_HINTS{input=False}` in `configure_x11_properties`. The empty input region is now also set in `new()` rather than on first show.

**Lesson**: X11 window properties that must influence the WM's focus decision must be set before the window is mapped, not in asynchronous post-map callbacks. Use `WidgetExt::realize()` to force surface creation without mapping, then set all Xlib properties synchronously before the first `set_visible(true)`.

## 11. Pre-filtering CLI arguments

**Symptom**: `rologlyphex --config -v` would strip `-v` from the argument list, then `--config` would fail with "missing path argument" — confusing because `-v` was intended as the config path value (pathological case), but the real issue is that pre-filtering changes argument positions.

**What was tried**:
- Scanning all args for `--verbose`/`-v`, filtering them out, then parsing the remaining args positionally

**Root cause**: Pre-filtering arguments before positional parsing changes the relationship between flags and their values. A flag's value might match the name of another flag.

**Resolution**: Single-pass parsing — `--verbose`/`-v` is handled as a case in the main `match` alongside other flags, consuming no additional arguments.

**Lesson**: Never pre-filter arguments from a list before positional parsing. Parse in a single pass where each flag consumes its own arguments from the iterator. This is why argument parsing libraries exist, but even hand-rolled parsers should follow this pattern.

## 12. `SupplementaryGroups` in a systemd user service fails with "Operation not permitted"

**Symptom**: The service failed to start with `Failed at step GROUP spawning /usr/local/bin/rologlyphex: Operation not permitted`.

**What was tried**:
- `SupplementaryGroups=keyd` in the `[Service]` section of a systemd **user** service

**Root cause**: User services run without `CAP_SETGID`. The `SupplementaryGroups` directive requires that capability to call `setgroups()`. System services (run by PID 1 as root) have it; user services don't. This is a fundamental systemd constraint, not a configuration error.

**Resolution**: Removed `SupplementaryGroups=keyd` from the service file. The user service inherits whatever groups the user has at login. The user must be in the `keyd` group at the OS level (`sudo usermod -aG keyd $USER`, then log out and back in). After that, the service inherits the group without any special directive.

**Lesson**: Never use `SupplementaryGroups` in a systemd user service — it will always fail. Group membership for user services must come from the user's login session. For keyd group access, `usermod -aG keyd $USER` + re-login is the only correct path.

## 13. Low X11 keycodes (near min_kc) silently swallow XTest key events

**Symptom**: A specific emoji (😬, U+1F62C) never appeared when typed — pressing the button produced no output. Sibling emoji (😐, 😨) in the same layer worked correctly.

**What was tried**:
- Verified UTF-8 encoding in config file (correct)
- Verified socket path discovery (correct — other emoji worked)
- Verified `unicode_to_keysym()` output (correct keysym 0x1001F62C)

**Root cause**: `XTyper::open()` scanned keycodes from `max_kc` down to `min_kc` (`.rev()`) and pushed free keycodes in that order — so the Vec was `[max_free, ..., min_free]`. `remap_and_type()` used `last()`, which yielded `min_free` (typically keycode 8 on Linux/evdev). Keycode 8 is X11's minimum valid keycode and while it appears free (all NoSymbol), `XTestFakeKeyEvent` with keycode 8 is silently ignored by the X server. The other emoji (😐, 😨) had been mapped in a previous daemon session and were reclaimed into the cache at startup — they never went through `remap_and_type`. 😬 had never been mapped, so it fell through to `remap_and_type` and received keycode 8.

**Resolution**: Changed the iteration from `(min_kc..=max_kc).rev()` to `min_kc..=max_kc` (ascending). Now `free_keycodes` is `[min_free, ..., max_free]` and `last()` yields the highest free keycode (e.g., 255). High keycodes are reliably unused by hardware and processed correctly by XTest.

**Lesson**: When allocating scratch X11 keycodes for `XChangeKeyboardMapping` + `XTestFakeKeyEvent`, always prefer high keycodes (near `max_kc`). Low keycodes near `min_kc` (8 on Linux) have special status and may be silently discarded by the X server when used with XTest.

## 14. Multimedia keysyms falsely reclaimed as rologlyphex emoji mappings

**Symptom**: After the fix for anti-pattern #13, some keycodes occupied by multimedia keys were reclaimed as "our" emoji mappings. The condition `first_sym >= 0x01000000` matched multimedia keysyms (e.g., `0x1008FF..` XF86 keysyms) which are in that range, causing those keycodes to be added to the cache with wrong keysym→keycode associations.

**Root cause**: The Unicode keysym range `0x01000000 + codepoint` for codepoints 0–10FFFF spans `0x01000000`–`0x0110FFFF`. XF86 multimedia keysyms use values like `0x1008FF02` (XF86Brightness) which fall above `0x0110FFFF` and are NOT Unicode keysyms — they are vendor-specific. The original check `first_sym >= 0x01000000` matched both.

**Resolution**: Tightened the reclaim condition to `first_sym >= 0x01000000 && first_sym <= 0x0110FFFF`, matching only valid Unicode keysyms.

**Lesson**: The `0x01000000 + codepoint` keysym encoding covers exactly `0x01000000`–`0x0110FFFF` (Unicode codepoints 0–10FFFF). Always use both bounds when checking for Unicode keysyms. Keysyms above `0x0110FFFF` are vendor-specific (XF86, multimedia, etc.) and must not be treated as Unicode.

## 15. GTK4 single-instance mechanism causes silent exit code 0 when another instance is running

**Symptom**: The systemd user service exits with `status=0/SUCCESS` after ~10ms, auto-restarts, and exits again in a tight loop. No error output appears in the journal. `systemctl status` shows clean exits with `code=exited, status=0/SUCCESS`.

**What was tried**:
- Checking service environment variables (`DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`) — all correct
- Checking binary timestamps — binary was current
- Running the binary manually to capture stderr — the manual run also exited quickly with only the first few debug lines

**Root cause**: GTK4's `Application` uses the `application_id` (`com.extollit.rologlyphex`) to register on the D-Bus session bus as a single-instance application. When a second instance starts with the same ID, GTK routes the `activate` signal to the already-running primary instance and the new instance exits immediately with code 0 — cleanly and silently. In this case, a manual debugging run (`rologlyphex --verbose &`) had been backgrounded and was still running, holding the D-Bus name. Every service restart attempt yielded to it.

**Resolution**: `pgrep -a rologlyphex` revealed the stale background process. `busctl --user list | grep extollit` confirmed it held the D-Bus registration. Killing the stale process allowed the service to start normally.

**Lesson**: When a GTK4 daemon exits with code 0 immediately after startup with no error output, the first thing to check is another instance holding the same `application_id`. Use `pgrep -a <name>` and `busctl --user list | grep <app-id>` to find it. Never leave manual debugging runs of a GTK4 daemon backgrounded while also trying to run the systemd service.

## 17. keyd hard-limits `command()` calls to 64 per config file — excess bindings silently dropped

**Symptom**: Some emoji buttons stopped typing characters. Entire layouts appeared non-functional. keyd logs showed `WARNING: /etc/keyd/macropad.conf:N: max commands (64), exceeded` for affected lines but continued starting normally.

**Root cause**: keyd enforces a compile-time limit of 64 `command()` invocations per config file. With 8 emoji layouts × 10 `command(rologlyphex type …)` bindings = 80 total, the last 16 entries were silently dropped. Affected keys either did nothing or fell through to an unrelated base-layer binding. The warning is easy to miss because keyd still starts and most functionality works.

**Resolution**: Reduced emoji layout count from 8 to 6 (60 command() calls, within the limit). The specific limit depends on the keyd version; `MAX_COMMANDS` may differ between releases. Check `journalctl -u keyd` after every config change that adds `command()` bindings.

**Lesson**: keyd silently truncates `command()` bindings beyond 64 — it does not error, crash, or warn prominently. After any keyd config edit that adds `command()` entries, grep the journal for "max commands" before assuming the config is correct. If the limit must be raised, patch keyd's source (`MAX_COMMANDS` constant). Alternatively, restructure the config to use the `[main]` base layer for shared bindings so emoji layouts don't each need their own `command()` entries (requires a `rologlyphex type-nth N` client command). (Superseded: keyd was later retired entirely — see ANTI-PATTERNS #21–23 and TECH.md.)

## 18. A full-keyboard virtual device consumes nearly all X11 keycodes, leaving only ~10 free for XTest remapping

**Symptom**: After adding a full-keyboard virtual device (e.g., one created by a keyboard management daemon for a secondary keyboard) to the keyd config, `rologlyphex type` started failing with `Error: no free keycodes for keysym 0x101XXXX` after only 10 unique emoji. Previously dozens of emoji worked without error. `python3 -c "import subprocess; ..."` (xmodmap analysis) confirmed only 10 all-NoSymbol keycodes remained out of 248 total.

**Root cause**: A full-keyboard virtual device registers an enormous keymap covering nearly all 248 X11 keycodes (8–255). Of these, ~238 are occupied by the device's keys and multimedia functions. This leaves only ~10 truly free (all-NoSymbol) keycodes for XTyper's remapping pool. Previously (with only the macropad and a standard keyboard), ~100 free keycodes were available.

**Resolution**: Replaced the unbounded free-keycode pool with an LRU eviction cache in `XTyper`. When `free_keycodes` is exhausted, the least-recently-used emoji's keycode is evicted and reused for the new emoji. The evicted emoji incurs the ~30ms remap penalty again on next use; the current active emoji are always fast. Pool size is determined at startup by `scan_keycodes()` — with 10 free keycodes, 10 emoji can be "warm" simultaneously.

**Lesson**: Adding a high-keycode-density device (such as a full-keyboard virtual device from a keyboard management daemon) can reduce the X11 free keycode pool to near-zero. `XTyper::open()`'s free keycode scan gives the true pool size; check it with `python3` + `xmodmap -pk` if emoji typing fails. The LRU fix makes the pool size irrelevant for correctness — any number of unique emoji work, just with amortized remap cost.

## 19. EEPROM chord modifiers (Ctrl+Alt+Shift) bleed through keyd virtual keyboard, corrupting XCompose sequences

**Symptom**: After adding a secondary full-keyboard device to the combined keyd config, all `macro()` bindings on the macropad (arrow/symbol layouts) produced garbled output. `xev` showed `Ctrl+Shift+Meta+Cancel+asciicircum+parenright+G` instead of the expected bare `Cancel+6+0+g`. Layouts using `command(rologlyphex type …)` were unaffected.

**What was tried**:
- Restarting the input method daemon without its X11 input method bridge — no change
- Verifying `keyd.compose` entry — correct (`<Cancel> <6> <0> <g> : "≠"`)
- Running `keyd monitor` — showed only `cancel 6 0 g` (no modifiers from keyd's virtual keyboard output)

**Root cause**: The macropad EEPROM was originally flashed with `Ctrl+Alt+Shift+F13-F18` chord mappings. When a button is pressed, the device emits the modifier keys (Ctrl, Alt, Shift) as separate key events before the F-key. keyd intercepts the F-key and correctly emits the `macro()` compose sequence (`cancel 6 0 g`), but the modifier events pass through keyd's virtual keyboard to X11 unfiltered. Xlib's compose engine then sees the digit keysyms in their Shift variants (`asciicircum` instead of `6`, `parenright` instead of `0`, `G` instead of `g`), which don't match the compose entries in keyd.compose.

Previously, `alt = noop` / `ctrl = noop` / `shift = noop` bindings in each keyd layout section absorbed the leaking modifiers. These noops were removed when a full-keyboard device was added to the combined config (they would have also suppressed normal keyboard modifier input, breaking typing). The `command(rologlyphex type …)` path was immune because XTest key synthesis doesn't go through XCompose.

The `mapping.yaml` file already showed bare `f13-f18` without chord modifiers, but this was a documentation artifact — it used invalid X11 keysym names (`xf86tools`, `xf86launch5-9`) rather than the tool-recognized names (`f13`-`f18`), and had never been validated or flashed. The EEPROM still contained the original chord mapping.

**Resolution**: Corrected `mapping.yaml` to use valid `ch57x-keyboard-tool` key names (`f13`-`f18`) and reflashed the macropad EEPROM. Bare F-keys carry no modifiers, so nothing bleeds through keyd. All `macro()` layouts immediately started working correctly.

**Lesson**:
- The EEPROM is the ground truth for what a device sends — not `mapping.yaml`. Verify the EEPROM's actual content matches intent; `mapping.yaml` can diverge silently if edited without reflashing.
- Modifier chord keys on a keyd-intercepted device bleed through to X11 even when keyd intercepts the trigger key. Do not use Ctrl/Alt/Shift chords as macropad triggers in a combined config that also includes a full keyboard — either suppress the modifiers with `noop` bindings (conflicts with full keyboard use) or eliminate them from the EEPROM entirely.
- `ch57x-keyboard-tool` key names (`f13`–`f18`) differ from X11 keysym names (`XF86Tools`, `XF86Launch5-9`). Always validate `mapping.yaml` with `ch57x-keyboard-tool validate` before treating it as flash-ready.
- When diagnosing XCompose failures, compare `keyd monitor` output (what keyd emits) with `xev` output (what X11 receives). Any keys visible in `xev` but absent from `keyd monitor` originate outside keyd — in this case, the EEPROM chord modifiers.

## 20. Watching the keyd config file with inotify to detect reloads

**Symptom**: After `keyd reload`, the overlay immediately displayed a blank or stale window before any key was pressed.

**What was tried**:
- `inotify` (via the `notify` crate) to watch the keyd config file for writes, triggering an immediate reparse and setting `config_reload_flag` to show the overlay

**Root cause**: inotify fires the moment the config file's write syscall completes — before keyd has read the updated file and applied the new config. The reparse at that instant may read a partially-written file or may read the correct file but then display the overlay before keyd is ready, so the first button press after a reload types the wrong character until keyd catches up. More critically, `config_reload_flag = true` triggers the GTK poll to call `show_layout()` immediately, before the layout map has been repopulated with the new content. If the parse returns an empty or partial map, the overlay shows blank.

The inotify path adds nothing correct here. When the user edits the keyd config and runs `keyd reload`, the authoritative signal that keyd has finished reloading is the `/main` IPC event — keyd sends this to all `IPC_LAYER_LISTEN` clients after it has successfully parsed and applied the new configuration.

**Resolution**: Removed the inotify watcher entirely. Config reparse is now triggered exclusively from `handle_layout_event` when it receives a `/main` IPC event. The `notify` crate dependency, `watch_config_file`, `debounced_reparse`, `try_claim_reparse`, and the shared `Arc<Mutex<Instant>>` debounce state were all removed.

**Lesson**: For daemons that wrap a reloadable service (keyd, nginx, systemd, etc.), do not use filesystem events to detect when a reload is complete. The file write and the service reload are two separate events; the file write always comes first, sometimes by hundreds of milliseconds. Use the service's own notification mechanism (IPC socket event, D-Bus signal, PID file update, or a purpose-built readiness protocol) to know when the reload is actually done.
