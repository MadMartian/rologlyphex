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

**Resolution**: Fixed-size window (default 600x275, configurable via `--size WxH`). Position calculated as `monitor_x + monitor_width - window_width - margin` using values known at construction time.

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

**Symptom**: Emoji characters typed via clipboard paste appeared as `^V^V^V...` in Konsole.

**What was tried**:
- Implementing full X11 clipboard ownership (claim CLIPBOARD, send Ctrl+V via XTest, serve SelectionRequest events)
- Hardening for multi-request protocol (TARGETS then UTF8_STRING)
- Adding timeout handling, stale event draining, refusal for unsupported targets
- Supporting obsolete protocol (property=None)

**Root cause**: Terminal emulators interpret Ctrl+V as "insert next character literally" (a readline/VT convention), not as "paste from clipboard." This is fundamental to how terminals work and cannot be detected or worked around at the X11 level.

**Resolution**: Reverted the non-BMP clipboard gate. All characters now use the keysym remap path. Clipboard code was eventually removed entirely (shelved in git history; see `sdd/ISSUES.md` entry D for full investigation notes).

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
