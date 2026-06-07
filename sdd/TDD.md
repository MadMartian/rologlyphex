# Test-Driven Development Rubrics

| # | Functional area | Rubrics    |
|---|----------------|------------|
| 1 | CLI dispatch | 1.1 – 1.11 |
| 2 | Config parsing | 2.1 – 2.15 |
| 3 | keyd IPC | 3.1 – 3.5  |
| 4 | Overlay window | 4.1 – 4.14 |
| 5 | Unicode input synthesis | 5.1 – 5.7  |
| 6 | Socket IPC | 6.1 – 6.7  |
| 7 | Config hot reload | 7.1 – 7.7  |
| 8 | App configuration | 8.1 – 8.4  |
| 9 | Miscellaneous | 9.1        |

## 1. CLI dispatch

### 1.1 Daemon mode is default
- **Given** the binary is invoked with `--config /path/to/config`
- **When** it starts
- **Then** it runs as a daemon (GTK main loop, IPC listener, socket server)

### 1.2 Type subcommand enters client mode
- **Given** the binary is invoked with `type <char>`
- **When** it starts
- **Then** it connects to the daemon socket, sends the character, and exits

### 1.3 Missing config flag is an error
- **Given** the binary is invoked with no arguments and no `type` subcommand
- **When** it starts
- **Then** it prints an error and usage message and exits with non-zero status

### 1.4 Missing type argument is an error
- **Given** the binary is invoked with `type` and no character argument
- **When** it starts
- **Then** it prints a usage message and exits with non-zero status

### 1.5 Size flag sets overlay width
- **Given** the binary is invoked with `--size 800x300`
- **When** the daemon starts
- **Then** the overlay window is created with width 800; height is content-driven and the `300` component is ignored

### 1.6 Size flag rejects non-positive dimensions
- **Given** the binary is invoked with `--size -1x0`
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

## 2. Config parsing

### 2.1 Layout sections are detected
- **Given** a keyd config containing `[arrows:layout]`
- **When** the config is parsed
- **Then** a layout named "arrows" is present in the result map

### 2.2 Non-layout sections are ignored
- **Given** a keyd config containing `[ids]` and `[main]` (no `:layout` suffix)
- **When** the config is parsed
- **Then** neither appears in the result map

### 2.3 Label comment overrides derived name
- **Given** the line `# label: Fire & Stars` immediately precedes `[fire:layout]`
- **When** the config is parsed
- **Then** the layout's display label is "Fire & Stars", not "Fire"

### 2.4 Non-adjacent label comment is ignored
- **Given** a `# label: Custom` comment separated from its section header by a blank line
- **When** the config is parsed
- **Then** the layout uses the identifier-derived label, not "Custom"

### 2.5 Identifier derives to title case
- **Given** a layout section `[my_arrows:layout]` with no label comment
- **When** the config is parsed
- **Then** the display label is "My Arrows"

### 2.6 setlayout bindings are excluded from buttons
- **Given** a layout section containing `f16 = setlayout(next)` and `f13 = command(rologlyphex type X)`
- **When** the config is parsed
- **Then** the layout's button list contains only the character from f13, not setlayout

### 2.7 noop bindings are excluded from buttons
- **Given** a layout section containing `alt = noop`
- **When** the config is parsed
- **Then** the noop binding does not appear in the button list

### 2.8 macro() wrapper is stripped
- **Given** a binding `f13 = macro(→)`
- **When** the display character is extracted
- **Then** the result is "→"

### 2.9 command(rologlyphex type ...) wrapper is stripped
- **Given** a binding `f13 = command(rologlyphex type 🔥)`
- **When** the display character is extracted
- **Then** the result is "🔥"

### 2.10 Buttons appear in config-file order
- **Given** a layout with f13=A, f14=B, f15=C in that order
- **When** the config is parsed
- **Then** the button list is [A, B, C] in that order

### 2.11 Bare unicode character is extracted
- **Given** a binding `f13 = →` (no `macro()` wrapper)
- **When** the display character is extracted
- **Then** the result is "→"

### 2.12 macro2() wrapper is stripped
- **Given** a binding `f13 = macro2(400, 50, →)`
- **When** the display character is extracted
- **Then** the result is "→"

### 2.13 Button label comment overrides display character
- **Given** the line `# label: ➡` immediately precedes `f13 = macro(→)`
- **When** the config is parsed
- **Then** the button's display character is "➡", not "→"

### 2.14 Button label takes first character only
- **Given** the line `# label: Right Arrow` immediately precedes a binding
- **When** the config is parsed
- **Then** the button's display character is "R" and a warning is logged

### 2.15 Layout header must end with :layout exactly
- **Given** a section header `[name:layout-extra]`
- **When** the config is parsed
- **Then** it is not treated as a layout section

## 3. keyd IPC

### 3.1 Daemon connects to keyd socket
- **Given** keyd is running and `/var/run/keyd.socket` exists
- **When** the daemon starts
- **Then** it connects and sends an `IPC_LAYER_LISTEN` message (4112 bytes)

### 3.2 Layout change events update current layout
- **Given** the IPC connection receives `/arrows\n`
- **When** the event is processed
- **Then** the shared current layout becomes "arrows"

### 3.3 Non-layout events are ignored
- **Given** the IPC connection receives `+modifier\n`
- **When** the event is processed
- **Then** the current layout is unchanged

### 3.4 /main event triggers config re-parse
- **Given** the IPC connection receives `/main\n`
- **When** the event is processed
- **Then** the keyd config file is re-parsed and the layout map is updated

### 3.5 Reconnection on socket closure
- **Given** keyd is restarted and the socket closes
- **When** the IPC thread detects the closure
- **Then** it reconnects to the new socket after a brief delay

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

### 4.6 Overlay positioned at top-right of rightmost monitor
- **Given** multiple monitors are connected
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

### 6.3a Socket discovery identifies active seat0 session
- **Given** multiple users are logged in, each with a running daemon
- **When** `rologlyphex type` is invoked as root
- **Then** the socket belonging to the user with an active X11 session on seat0 is used

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

## 7. Config hot reload

### 7.1 Config change updates overlay content
- **Given** a layout's character binding is changed in the keyd config
- **When** `keyd reload` is run
- **Then** the daemon reflects the updated configuration — new characters type correctly and the overlay shows updated content — without restarting

### 7.2 Config re-parse is debounced
- **Given** the keyd config file is written multiple times within 200ms
- **When** the inotify watcher fires
- **Then** the config is re-parsed only once

### 7.3 Adding a new layout is picked up
- **Given** a new `[name:layout]` section is added to the keyd config
- **When** the config is re-parsed
- **Then** the new layout appears in the overlay when navigated to

### 7.4 Updating the group configuration
- **Given** the keyd config file's group information was changed
- **When** the config is reparsed
- **Then** the overlay updates to reflect the new group configuration

### 7.5 Loading group configuration
- **Given** the keyd config file's contains group information
- **When** the config is parsed or reparsed
- **Then** the group configuration is parsed and loaded from the `[ids]` section

### 7.6 Anonymous group organization
- **Given** the `keyd` configuration is parsed
- **When** some keys do not belong to any groups 
- **Then** the groupless keys appear in a separate group without a label

### 7.7 Rendering anonymous groups
- **When** groupless keys are rendered in the overlay
- **Then** the groupless group is rendered first on-top, preceding named groups

## 8. App configuration

### 8.1 Default values used when no config exists
- **Given** no `config.toml` exists
- **When** the daemon starts with valid CLI args
- **Then** it starts successfully using CLI values

### 8.2 Config file values override defaults
- **Given** a `config.toml` with `timeout = 5000` and no `--timeout` CLI arg
- **When** the daemon starts
- **Then** it uses a 5000ms dismiss timeout

### 8.3 CLI args override config file values
- **Given** a `config.toml` with `timeout = 5000` and `--timeout 2000` CLI arg
- **When** the daemon starts
- **Then** it uses a 2000ms dismiss timeout

### 8.4 Graceful handling of malformed config
- **Given** a malformed `config.toml`
- **When** the daemon starts with valid CLI args
- **Then** it logs a warning, falls back to defaults for missing values, and starts successfully

## 9. Miscellaneous

### 9.1 Crash logging
- **Given** the daemon is running
- **When** it encounters an unrecoverable error
- **Then** a message identifying the failure location and cause is written to the system log
