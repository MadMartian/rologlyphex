# Test-Driven Development Rubrics

| # | Functional area | Rubrics    |
|---|----------------|------------|
| 1 | CLI dispatch | 1.1 – 1.11 |
| 2 | Layers config | 2.1 – 2.7  |
| 3 | Key input & layer ring | 3.1 – 3.7  |
| 4 | Overlay window | 4.1 – 4.21 |
| 5 | Unicode input synthesis | 5.1 – 5.8  |
| 6 | Socket IPC | 6.1 – 6.7  |
| 7 | App configuration | 7.1 – 7.4  |
| 8 | Miscellaneous | 8.1        |
| 9 | Non-BMP emoji routing | 9.1 – 9.6  |

## 1. CLI dispatch

### 1.1 Daemon mode is default
- **Given** the binary is invoked with no `type`/`show` subcommand
- **When** it starts
- **Then** it runs as a daemon (GTK main loop, key-grab thread, socket server)

### 1.2 Type subcommand enters client mode
- **Given** the binary is invoked with `type <char>`
- **When** it starts
- **Then** it connects to the daemon socket, sends the character, and exits

### 1.3 Daemon requires a usable layers config
- **Given** no usable `layers.toml` (missing, or an empty navigation ring)
- **When** the daemon loads it (from `--config`/`layers` or the default path)
- **Then** it prints an error and exits with non-zero status

### 1.4 Missing type argument is an error
- **Given** the binary is invoked with `type` and no character argument
- **When** it starts
- **Then** it prints a usage message and exits with non-zero status

### 1.5 Size flag sets overlay width
- **Given** the binary is invoked with `--size 800`
- **When** the daemon starts
- **Then** the overlay window is created with width 800; height is content-driven

### 1.6 Size flag rejects non-positive dimensions
- **Given** the binary is invoked with `--size -1`
- **When** it starts
- **Then** it prints an error and exits with non-zero status

### 1.7 Type accepts exactly one Unicode character
- **Given** the binary is invoked with `type →`
- **When** the character is parsed
- **Then** exactly one character `→` is sent to the daemon

### 1.8 Type with multiple characters uses first only
- **Given** the binary is invoked with `type abc`
- **When** the character is parsed
- **Then** only `a` is sent to the daemon and a warning is logged

### 1.9 Verbose flag enables debug output
- **Given** the binary is invoked with `--verbose --config /path/to/config`
- **When** the daemon starts
- **Then** debug messages are printed to stderr

### 1.10 Verbose flag is off by default
- **Given** the binary is invoked with `--config /path/to/config` (no `--verbose`)
- **When** the daemon runs
- **Then** no debug messages are printed to stderr

### 1.11 X11 backend is required
- **Given** the daemon starts under a Wayland-only session
- **When** the GDK backend is detected
- **Then** it prints an error mentioning `GDK_BACKEND=x11` and exits with non-zero status

## 2. Layers config

### 2.1 Aliases resolve to logical names
- **Given** `[keys]` maps `F13 = "SL1"` and a layer binds `SL1 = "←"`
- **When** the config is loaded
- **Then** typing for that layer/key yields `←`, keyed by the logical name `SL1` (not the F-key)

### 2.2 Physical key references resolve to their alias
- **Given** `[keys]` maps `F13 = "SL1"` and a layer binding is written `F13 = "←"`
- **When** the config is loaded
- **Then** it resolves to the logical name `SL1`; F-keys are never internal identifiers

### 2.3 Navigation defaults and overrides
- **Given** the `[navigation]` section is absent
- **When** the config is loaded
- **Then** prev/next default to raw `F16`/`F18`; an explicit `[navigation]` overrides them with raw F-keys, which must differ

### 2.4 Navigation and keys are disjoint
- **Given** a key appears both as a `[navigation]` key and a `[keys]` alias
- **When** the config is loaded
- **Then** loading fails with an error (the two sections must partition the keyspace)

### 2.5 Global groups apply per layer in render order
- **Given** a `[[groups]]` section assigning keys to groups
- **When** a layer defines only some of those keys
- **Then** the overlay renders each group (in declaration order) containing only that layer's present keys, in the group's key order

### 2.6 Ungrouped keys form the anonymous group first
- **Given** a layer has typeable keys not listed in any group
- **When** the overlay model is built
- **Then** those keys form a single unlabeled group rendered before the named groups

### 2.7 Layer-order validation and default labels
- **Given** `layer_order` with unknown entries, or that resolves to no layers
- **When** the config is loaded
- **Then** unknown entries are skipped with a warning and an empty ring is a fatal error; a layer with no `label` defaults to its name in Title Case

## 3. Key input & layer ring

### 3.1 Knob navigates the layer ring
- **Given** the daemon has grabbed the navigation keys and is on a layer
- **When** the knob next/previous key is pressed
- **Then** the active layer advances to the next/previous entry in `layer_order`, wrapping at the ends, and the overlay shows the new layer

### 3.2 Button types the active layer's glyph on release
- **Given** the active layer binds a logical key to a glyph
- **When** the corresponding physical key is pressed and released
- **Then** the glyph is typed into the focused window on key **release** (not press, so the active grab does not swallow the injection)

### 3.3 Macro keys absent from the X keymap are still grabbed
- **Given** function keys F19–F24 have no keysym in the X keymap
- **When** the daemon resolves keycodes to grab
- **Then** it derives their keycodes from the evdev codes (191–202) and grabs them, so the secondary keyboard's macro keys work

### 3.4 Keymap remap is per layer change, never per keypress or on navigation
- **Given** the user spins the knob through several layers
- **When** navigation occurs
- **Then** no `XChangeKeyboardMapping` is issued during navigation; the active layer's glyphs are remapped in a single call only when that layer is first typed in (lazy) or after the knob settles (debounce)

### 3.5 Lazy mode shows "Please Wait" during the remap
- **Given** `remap_mode = "lazy"` and a freshly-entered layer
- **When** the first key in that layer is pressed
- **Then** a "Please Wait" overlay is shown centered across all monitors while the keymap is rebuilt, then hidden, and the glyph types

### 3.6 Debounce mode remaps silently after the knob settles
- **Given** `remap_mode = "debounce"`
- **When** the knob stops for `nav_settle_ms`
- **Then** the active layer is remapped in the idle gap with no indicator, so the first keypress is immediate

### 3.7 Navigation and typeable keyspaces are disjoint
- **Given** a layers config where a key is both a `[navigation]` key and a `[keys]` alias
- **When** the config is loaded
- **Then** loading fails with a clear error (the two sections must partition the keyspace)

## 4. Overlay window

### 4.1 Overlay appears on layout change
- **Given** the daemon is running and the overlay is hidden
- **When** the current layout changes
- **Then** the overlay becomes visible showing the layout's label and button legends

### 4.2 Overlay updates on rapid layout changes
- **Given** the overlay is visible showing layout A
- **When** the layout changes to B before the dismiss timer expires
- **Then** the overlay updates in place to show layout B and the dismiss timer resets

### 4.3 Overlay auto-dismisses after timeout
- **Given** the overlay is visible and no further layout changes occur
- **When** the configured timeout elapses
- **Then** the overlay hides

### 4.4 Overlay does not steal focus
- **Given** a text editor has focus (including on the very first overlay appearance after daemon start)
- **When** the overlay appears
- **Then** the text editor retains focus and remains typeable

### 4.5 Overlay is click-through
- **Given** the overlay is visible over a portion of another window
- **When** the user clicks on the overlay's screen area
- **Then** the click passes through to the window underneath

### 4.6 Overlay positioned at top-right of rightmost monitor by default
- **Given** multiple monitors are connected and no `monitor`/`corner` is configured
- **When** the overlay appears
- **Then** it is anchored to the top-right corner of the monitor with the greatest x + width

### 4.7 Long title reduces to smaller font
- **Given** a layout label that exceeds the overlay's available width at 72px
- **When** the overlay displays that layout
- **Then** the title renders at 48px (2/3 of normal size)

### 4.8 Very long title shows ellipsis
- **Given** a layout label that exceeds the overlay's available width even at 48px
- **When** the overlay displays that layout
- **Then** the title is truncated with a trailing ellipsis

### 4.9 Overlay visible on all virtual activities
- **Given** the daemon is running on a desktop environment with multiple virtual activities
- **When** the user switches to a different activity and triggers a layout change
- **Then** the overlay appears on that activity (not confined to the launch activity)

### 4.10 Overlay visible on all virtual desktops
- **Given** the daemon is running with multiple virtual desktops configured
- **When** the user switches to a different virtual desktop and triggers a layout change
- **Then** the overlay appears on that desktop (not confined to the launch desktop)

### 4.11 Button legends wrap when they overflow a single row
- **Given** a layout has more button legends than fit in a single row at the configured window width
- **When** the overlay appears
- **Then** all button legends are visible, arranged across multiple rows

### 4.12 Presenting the overlay window
- **Given** the daemon is running and the overlay is hidden
- **When** the user requests to show the overlay window
- **Then** the overlay appears with the current layout's label and button legends

### 4.13 Refreshing the overlay window
- **Given** the daemon is running and the overlay is shown
- **When** the user requests to show the overlay window
- **Then** the timer to dismiss the overlay resets

### 4.14 Request to show the overlay but the daemon is not running
- **Given** the daemon is not running and the overlay is hidden
- **When** the user requests to show the overlay window
- **Then** the overlay is not shown and an error message is returned with a non-zero status

### 4.15 Configured corner anchors the overlay
- **Given** `corner` is set to `bottom-left`
- **When** the overlay appears on the target monitor
- **Then** it is anchored to the bottom-left corner, inset by the window margin, with its bottom edge offset by the measured window height

### 4.16 Configured monitor selects the display
- **Given** `monitor` matches a connected monitor's connector name (e.g. `DP-1`)
- **When** the overlay appears
- **Then** it is positioned on that monitor rather than the rightmost one

### 4.17 Unknown monitor falls back to rightmost
- **Given** `monitor` matches no connected monitor's connector, model, or index
- **When** the overlay appears
- **Then** a warning is printed and the overlay falls back to the rightmost monitor

### 4.18 Unrecognized corner falls back to top-right
- **Given** `corner` is set to a value that is not one of the four corners
- **When** the daemon resolves the corner preference
- **Then** a warning is printed and the corner defaults to top-right

### 4.19 App header is shown on both windows
- **Given** the overlay and the "Please Wait" window
- **When** they are displayed
- **Then** each shows a small header — the app icon followed by "Rologlyphex!" — with the rest of the content below it (the header degrades to text only if the SVG loader is unavailable)

### 4.20 Header hugs the configured corner; layer title is centered
- **Given** the overlay is configured for a corner (e.g. `bottom-left`)
- **When** the layer overlay is shown
- **Then** the app header aligns to that corner inside the window (horizontal side via alignment, top/bottom via placement) and the layer title is centered

### 4.21 "Please Wait" window is centered across all displays
- **Given** lazy remap mode triggers the "Please Wait" window
- **When** it is shown
- **Then** it is centered across the union bounding box of all monitors, distinct from the corner-aligned layer overlay

## 5. Unicode input synthesis

### 5.1 BMP character is typed via XTest
- **Given** the daemon receives "→" (U+2192) via the socket
- **When** it processes the character
- **Then** an XTest key event for the corresponding keysym is sent to the X server

### 5.2 First use of unmapped character remaps a keycode
- **Given** a character whose keysym has no existing keycode mapping
- **When** it is typed for the first time
- **Then** the daemon maps the keysym to an unused keycode via `XChangeKeyboardMapping` and sends the key event

### 5.3 Cached characters skip remapping
- **Given** a character that was previously remapped and cached
- **When** it is typed again
- **Then** the cached keycode is used directly with no `XChangeKeyboardMapping` call

### 5.4 Already-mapped characters use existing keycode
- **Given** a character whose keysym already has a keycode in the X server's mapping
- **When** it is typed
- **Then** the existing keycode is used with no remapping

### 5.5a Character types correctly into pkexec-elevated apps
- **Given** an X11 app started via `pkexec` has keyboard focus
- **When** `rologlyphex type →` is invoked for a character being used for the first time
- **Then** the correct character appears (not a garbled or wrong glyph)

### 5.5 Character appears in focused window
- **Given** a text editor has focus
- **When** `rologlyphex type →` is invoked
- **Then** "→" appears at the cursor position in the text editor

### 5.6 Characters type correctly when remapping is needed
- **Given** no prior character-to-keycode mappings exist from the daemon (e.g., first run or after an X server restart)
- **When** a character is typed that requires a new keycode assignment
- **Then** the character appears correctly in the focused window

### 5.7 Startup does not interfere with existing keyboard shortcuts
- **Given** the keyboard has multimedia or other special keys bound before the daemon starts
- **When** the daemon starts
- **Then** those keys continue to function normally

### 5.8 X protocol errors are non-fatal
- **Given** the daemon has installed its X error handlers (`xerror::install`)
- **When** an Xlib call triggers a recoverable protocol error (e.g. `BadAccess` from `XGrabKey` when another client holds a key, or `BadWindow` on a destroyed overlay)
- **Then** the daemon logs the error details and continues running rather than `exit()`-ing, and the daemon socket remains served

## 6. Socket IPC

### 6.1 Socket created on daemon startup
- **Given** the daemon starts
- **When** initialization completes
- **Then** a Unix socket exists at `$XDG_RUNTIME_DIR/rologlyphex.sock`

### 6.2 Stale socket is cleaned up
- **Given** a socket file exists from a previous crashed daemon
- **When** the daemon starts
- **Then** the stale socket is removed and a new one is created

### 6.3 Client discovers socket when run as root
- **Given** `rologlyphex type X` is invoked as root (no `XDG_RUNTIME_DIR`)
- **When** it looks for the daemon socket
- **Then** it finds it by scanning `/run/user/*/rologlyphex.sock`

### 6.4 Client fails gracefully when daemon is not running
- **Given** the daemon is not running and no socket exists
- **When** `rologlyphex type X` is invoked
- **Then** it prints an error and exits with non-zero status

### 6.5 Sequential type requests are all served
- **Given** three `rologlyphex type` commands arrive in rapid succession
- **When** the socket server processes them
- **Then** all three characters are typed in order

### 6.6 Server enforces single-character limit
- **Given** a client sends multiple characters over the socket
- **When** the server reads the data
- **Then** only the first character is typed and a warning is logged

### 6.7 Server enforces maximum read size
- **Given** a client sends more than 16 bytes over the socket
- **When** the server reads the data
- **Then** only the first 16 bytes are read and the connection is closed

## 7. App configuration

### 7.1 Default values used when no config exists
- **Given** no `config.toml` exists
- **When** the daemon starts with valid CLI args
- **Then** it starts successfully using CLI values

### 7.2 Config file values override defaults
- **Given** a `config.toml` with `timeout = 5000` and no `--timeout` CLI arg
- **When** the daemon starts
- **Then** it uses a 5000ms dismiss timeout

### 7.3 CLI args override config file values
- **Given** a `config.toml` with `timeout = 5000` and `--timeout 2000` CLI arg
- **When** the daemon starts
- **Then** it uses a 2000ms dismiss timeout

### 7.4 Graceful handling of malformed config
- **Given** a malformed `config.toml`
- **When** the daemon starts with valid CLI args
- **Then** it logs a warning, falls back to defaults for missing values, and starts successfully

## 8. Miscellaneous

### 8.1 Crash logging
- **Given** the daemon is running
- **When** it encounters an unrecoverable error
- **Then** a message identifying the failure location and cause is written to the system log

## 9. Non-BMP emoji routing

### 9.1 Emoji into a whitelisted Java/AWT app is delivered by clipboard paste
- **Given** a layer binds a key to a non-BMP glyph (e.g. 🔥, U+1F525), `[non_bmp].clipboard_apps` lists the focused application's window class, and that application is focused
- **When** the key is pressed and released
- **Then** the emoji appears intact at the cursor (surrogate pairs preserved), delivered via clipboard paste rather than the keysym path

### 9.2 Emoji into a non-whitelisted or non-AWT window uses the keysym path
- **Given** a non-BMP glyph is bound and the focused window's class is not listed in `[non_bmp].clipboard_apps` (e.g. a terminal or browser)
- **When** the key is pressed and released
- **Then** the glyph is delivered via the keysym path, with no clipboard paste and no synthetic Ctrl+V sent to that window

### 9.3 BMP glyphs never use the clipboard
- **Given** a BMP glyph is bound (e.g. ✅, U+2705), even while a whitelisted Java/AWT app is focused
- **When** the key is pressed and released
- **Then** it is typed via the keysym path and the system clipboard is left untouched

### 9.4 The prior clipboard is restored after an emoji paste
- **Given** the system clipboard holds some content and a non-BMP glyph is pasted into a whitelisted Java/AWT app
- **When** the paste completes
- **Then** the clipboard is restored to its prior content (best-effort)

### 9.5 Clipboard routing is off when unconfigured
- **Given** `[non_bmp].clipboard_apps` is empty or unset
- **When** any non-BMP glyph is pressed in any window
- **Then** it always takes the keysym path and no clipboard-owner activity occurs

### 9.6 Focus is evaluated at the moment of delivery
- **Given** a non-BMP glyph is pressed while a whitelisted Java/AWT app is focused
- **When** the glyph is delivered (on key release, after any pending keymap remap has completed)
- **Then** the clipboard-vs-keysym decision reflects the window focused at delivery time, not the window focused at an earlier layer switch
