# Schema Reference

This document defines rologlyphex's configuration file formats field-by-field. For how
these files fit into the architecture, see TECH.md.

## Layers config (`layers.toml`)

The glyph map and layer-ring order that rologlyphex owns. It assigns each button, per layer,
the glyph rologlyphex types and displays, and defines the navigation ring the knob cycles
through.

Default location: `~/.config/rologlyphex/layers.toml` (overridable; see the `layers` field
in the app config below). Parsed with `serde` + `toml` into the structures below.

### Vernacular: logical names, not function keys

Physical function keys (`F13`–`F24`) appear **only** in two places: the `[keys]` section,
where typeable keys are given logical names (e.g. `SL1`, `G6`), and the `[navigation]`
section, where the two layer-switching keys are named by **raw F-key code** (their prev/next
role is itself the mapping, so they are not aliased). Everywhere else — `groups`, `buttons` —
references **logical key names**, and all of rologlyphex's runtime state and logging speaks
the logical vernacular (typeable keys by logical name; navigation by its prev/next action).

`[keys]` and `[navigation]` **partition** the physical keyspace: a key listed in `[navigation]`
must not also be aliased in `[keys]`, and vice versa (a load-time error). The concept of
`F13`–`F24` does not survive past this file except as the grab boundary's two translation
inputs (`phys_to_logical` for typeable keys; the raw navigation keys).

References are matched case-insensitively; logical names are canonicalized to uppercase.

> **TOML ordering**: `layer_order` is a root scalar and must appear **before** the first
> table header (`[navigation]`, `[keys]`, `[[groups]]`, `[layers.…]`), or TOML will absorb it
> into the preceding table.

### Top level

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `layer_order` | `array<string>` | yes | Ordered layer ring of layer names. `navigation.prev` selects the previous entry, `navigation.next` the next; navigation wraps. Each entry must be a key in `layers`. |
| `navigation` | `Navigation` | no | The keys that cycle the layer ring. See below. |
| `keys` | `table<string, string>` | yes | Alias map: physical function key (`F13`–`F24`) → logical name. The only place F-keys appear. |
| `groups` | `array<Group>` | no | Global key→group assignments for overlay rendering. A key belongs to the same group in every layer, so grouping is defined once here, not per layer. |
| `layers` | `table<string, Layer>` | yes | Map of layer name → layer definition. Names are referenced by `layer_order` and surfaced to the overlay. |

### `[navigation]`

The two layer-switching keys, named by **raw function-key code** (`F13`–`F24`,
case-insensitive). These keys must not also appear in `[keys]`.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `prev` | `string` | no | Raw F-key that navigates to the previous layer. Default `F16` (the macropad knob CCW). |
| `next` | `string` | no | Raw F-key that navigates to the next layer. Default `F18` (knob CW). Must differ from `prev`. |

### `keys` (alias map)

Each entry names one physical typeable key. Physical keys must be `F13`–`F24`
(case-insensitive); each logical name must be unique. The navigation keys are **not** listed
here — they live in `[navigation]` as raw codes.

| Physical key | Source | Typical alias |
|--------------|--------|---------------|
| `F13`, `F14`, `F15` | macropad buttons | `SL1`, `SL2`, `SL3` |
| `F16` | macropad knob CCW | *(navigation — not aliased)* |
| `F17` | macropad knob press | `SL4` |
| `F18` | macropad knob CW | *(navigation — not aliased)* |
| `F19`–`F24` | secondary keyboard macro keys | `G1`–`G6` |

### `Group`

Groups are a physical property of the keys (e.g. the macropad buttons vs. the secondary keyboard macro keys),
independent of layer. The array order sets the order groups render in; the `keys` order sets
the order of buttons within a group.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `label` | `string` | yes | Group title shown in the overlay above its buttons. |
| `keys` | `array<string>` | yes | Logical key names in this group, in render order (e.g. `["G1","G2","G3","G4","G5","G6"]`). A key may appear in at most one group. |

A typeable key not listed in any group renders in an unlabeled group shown first.

### `Layer`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `label` | `string` | no | Human title shown in the overlay. Defaults to the layer name in Title Case (e.g. `right_arrows` → `Right Arrows`). |
| `buttons` | `table<string, string>` | no | Map of logical key name → glyph. The glyph is the character typed when the button is pressed and shown as its overlay legend (first `char` used if multiple). Navigation keys must not appear. Omitted/empty = a navigation-only layer. |

**Example:**
```toml
layer_order = ["main", "right_arrows", "symbols", "glyphs"]

[navigation]
prev = "F16"   # raw key codes; F16/F18 are the macropad knob (CCW/CW)
next = "F18"

[keys]
F13 = "SL1"
F14 = "SL2"
F15 = "SL3"
F17 = "SL4"
F19 = "G1"
F20 = "G2"
F21 = "G3"
F22 = "G4"
F23 = "G5"
F24 = "G6"

[[groups]]
label = "Macropad"
keys = ["SL1", "SL2", "SL3", "SL4"]

[[groups]]
label = "G1-6"
keys = ["G1", "G2", "G3", "G4", "G5", "G6"]

[layers.main]
label = "Left Arrows"
[layers.main.buttons]
SL1 = "←"
SL2 = "⇐"
SL3 = "↤"
SL4 = "↚"
G1 = "↜"
G6 = "⇦"
```

### Validation

- Every entry in `layer_order` must exist in `layers`; unknown names are skipped with a
  warning. An empty resulting ring is an error.
- A layer present in `layers` but absent from `layer_order` is loadable but unreachable by
  navigation (warned).
- `navigation.prev`/`navigation.next` must be raw function keys (`F13`–`F24`), default
  `F16`/`F18`, and must differ.
- `[keys]` and `[navigation]` must be disjoint: a key used for navigation may not also be
  aliased in `[keys]` (a load-time error).
- A `buttons` or `groups` reference that is neither a logical name nor an aliased physical key
  is ignored with a warning. A reference that resolves to a navigation key is rejected.
- A `groups` entry may reference keys not present in a given layer; only keys with a glyph in
  that layer render. A key listed in more than one group is assigned to the first.

## App config (`config.toml`)

Daemon settings loaded from `~/.config/rologlyphex/config.toml` (or `$XDG_CONFIG_HOME`).
All fields are optional; CLI flags override file values. Parsed into `settings::AppSettings`.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `layers` | `string` | no | Path to the `layers.toml` glyph map (above). Defaults to `~/.config/rologlyphex/layers.toml`. |
| `timeout` | `uint64` | no | Overlay auto-dismiss timeout in milliseconds. Default `3000`. |
| `size` | `string` | no | Overlay window width in pixels (height is content-driven). Default `600`. |
| `monitor` | `string` | no | Target monitor: connector name (e.g. `DP-1`), model name, or numeric index. Default: rightmost monitor. |
| `corner` | `string` | no | Overlay corner: `top-left`, `top-right`, `bottom-left`, `bottom-right`. Default `top-right`. |
| `nav_settle_ms` | `uint64` | no | Navigation debounce (debounce mode only): milliseconds of input quiet after the knob settles on a layer before the typist remaps for it. Lower = quicker remap but risks remapping mid-spin; higher = longer window where typing right after stopping waits. Default `160`. |
| `remap_mode` | `"lazy" \| "debounce"` | no | When the typist rebuilds the keymap for a newly-entered layer. `lazy`: on the first keypress in the layer, showing a "Please Wait" overlay during the (slow) remap. `debounce`: in the idle gap after the knob settles (no indicator). Default `lazy`. |
| `verbose` | `bool` | no | Enable debug logging. Default `false`. |
