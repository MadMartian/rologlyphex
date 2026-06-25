// Glyph map and layer-ring config owned by rologlyphex (replaces parsing the keyd
// config). Loaded from layers.toml; see sdd/SCHEMA.md for the file format.
//
// Vernacular: physical function keys (F13–F24) live ONLY in the config ([keys] aliases and
// the raw [navigation] keys) and at the grab boundary. Past that, the whole daemon — typing
// lookup, overlay model, layer logic, and logging — speaks only logical key names (e.g.
// "SL1", "G6") and the prev/next navigation actions. The grab boundary holds the only two
// F-key structures: `phys_to_logical` (typeable keys -> logical names) and `nav_prev`/
// `nav_next` (raw F-keys). [keys] and [navigation] partition the keyspace — a key may not be
// both, which is a load-time validation error.
//
// Logical names are canonical uppercase; config references are matched case-insensitively.

use crate::config::{ButtonGroup, ButtonLegend, LayoutInfo};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;

/// Every physical function key the two devices can emit, in canonical render order. Used
/// only to validate [keys] and to order ungrouped buttons via their logical names.
const ALL_FKEYS: &[&str] = &[
    "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

#[derive(Debug, Deserialize)]
struct GroupDef {
    label: String,
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LayerDef {
    label: Option<String>,
    #[serde(default)]
    buttons: HashMap<String, String>,
}

/// `[navigation]` section: which keys cycle the layer ring.
#[derive(Debug, Default, Deserialize)]
struct NavigationDef {
    prev: Option<String>,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    /// Physical F-key -> logical alias (e.g. F13 = "SL1"). The only place F-keys appear.
    #[serde(default)]
    keys: HashMap<String, String>,
    #[serde(default)]
    navigation: NavigationDef,
    layer_order: Vec<String>,
    #[serde(default)]
    groups: Vec<GroupDef>,
    layers: HashMap<String, LayerDef>,
}

/// Fully-resolved layer configuration. All key identifiers are logical names; the only
/// physical-key structure is `phys_to_logical` (the grab-thread translation table).
#[derive(Debug, Clone)]
pub struct LayersConfig {
    /// Layer names in navigation order. `nav_prev` selects previous, `nav_next` next (wrapping).
    pub layer_order: Vec<String>,
    /// Physical function key (e.g. "F16") that navigates to the previous layer. Raw F-key,
    /// disjoint from `[keys]`; lives at the grab boundary alongside `phys_to_logical`.
    pub nav_prev: String,
    /// Physical function key (e.g. "F18") that navigates to the next layer.
    pub nav_next: String,
    /// Physical function key -> logical name. The grab thread's sole F-key reference.
    pub phys_to_logical: HashMap<String, String>,
    /// Overlay model, keyed by layer name. Buttons are pre-grouped in render order.
    pub overlay: HashMap<String, LayoutInfo>,
    /// Typed character per (layer name, logical key name).
    pub typing: HashMap<String, HashMap<String, char>>,
}

impl LayersConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read layers config '{}': {}", path, e))?;
        Self::from_str(&content)
    }

    pub fn from_str(content: &str) -> Result<Self, String> {
        let raw: RawConfig = toml::from_str(content)
            .map_err(|e| format!("Failed to parse layers config: {}", e))?;

        // Build the physical->logical translation table from [keys].
        let mut phys_to_logical: HashMap<String, String> = HashMap::new();
        let mut logical_set: HashSet<String> = HashSet::new();
        for (phys, alias) in &raw.keys {
            let phys_u = phys.to_uppercase();
            if !ALL_FKEYS.contains(&phys_u.as_str()) {
                eprintln!("Warning: [keys] entry '{}' is not a function key F13–F24; ignoring", phys);
                continue;
            }
            let alias_u = alias.to_uppercase();
            if !logical_set.insert(alias_u.clone()) {
                eprintln!("Warning: logical name '{}' is assigned to more than one key; using the last", alias);
            }
            phys_to_logical.insert(phys_u, alias_u);
        }

        // Resolve a config reference (a logical name, or a physical F-key that has an alias)
        // to its logical name. Physical F-keys without an alias do not resolve — by design,
        // logical names are required past [keys].
        let resolve = |name: &str| -> Option<String> {
            let u = name.to_uppercase();
            if logical_set.contains(&u) {
                Some(u)
            } else if let Some(l) = phys_to_logical.get(&u) {
                Some(l.clone())
            } else {
                None
            }
        };

        // [navigation] uses raw function-key codes — the prev/next role is itself a hard
        // mapping, so navigation keys are not aliased. Default to the macropad knob (F16/F18).
        let nav_key = |val: &Option<String>, default: &str, field: &str| -> Result<String, String> {
            let k = val.as_deref().unwrap_or(default).to_uppercase();
            if !ALL_FKEYS.contains(&k.as_str()) {
                return Err(format!("navigation.{} '{}' is not a function key F13–F24", field, k));
            }
            Ok(k)
        };
        let nav_prev = nav_key(&raw.navigation.prev, "F16", "prev")?;
        let nav_next = nav_key(&raw.navigation.next, "F18", "next")?;
        if nav_prev == nav_next {
            return Err(format!("navigation.prev and navigation.next are the same key ({})", nav_prev));
        }
        // A key may not be both a navigation key and an aliased typeable key.
        for navk in [&nav_prev, &nav_next] {
            if let Some(alias) = phys_to_logical.get(navk) {
                return Err(format!(
                    "key {} is a navigation key but is also aliased as '{}' in [keys]; navigation and [keys] must be disjoint",
                    navk, alias
                ));
            }
        }

        // Logical names of typeable keys, in canonical (physical) order, for the anonymous
        // group rendering. Navigation keys are never aliased, so they don't appear here.
        let typeable_order: Vec<String> = ALL_FKEYS
            .iter()
            .filter_map(|f| phys_to_logical.get(*f))
            .cloned()
            .collect();

        // Pre-resolve global groups to logical names, preserving order.
        let resolved_groups: Vec<(String, Vec<String>)> = raw
            .groups
            .iter()
            .map(|g| {
                let keys = g
                    .keys
                    .iter()
                    .filter_map(|k| {
                        resolve(k).or_else(|| {
                            eprintln!("Warning: group '{}' references unknown key '{}'; ignoring", g.label, k);
                            None
                        })
                    })
                    .collect();
                (g.label.clone(), keys)
            })
            .collect();

        let mut overlay = HashMap::new();
        let mut typing = HashMap::new();

        for (name, layer) in &raw.layers {
            let label = layer.label.clone().unwrap_or_else(|| derive_label(name));

            let mut key_to_char: HashMap<String, char> = HashMap::new();
            let mut key_to_glyph: HashMap<String, String> = HashMap::new();
            for (raw_key, glyph) in &layer.buttons {
                let logical = match resolve(raw_key) {
                    Some(l) => l,
                    None => {
                        eprintln!("Warning: layer '{}' button '{}' is not a known key or alias; ignoring", name, raw_key);
                        continue;
                    }
                };
                match crate::first_char_of(glyph, "layer glyph") {
                    Some(ch) => {
                        key_to_char.insert(logical.clone(), ch);
                        key_to_glyph.insert(logical, glyph.clone());
                    }
                    None => {
                        eprintln!("Warning: layer '{}' button '{}' has an empty glyph; ignoring", name, raw_key);
                    }
                }
            }

            overlay.insert(
                name.clone(),
                LayoutInfo {
                    label,
                    groups: organize_groups(&resolved_groups, &typeable_order, &key_to_glyph),
                },
            );
            typing.insert(name.clone(), key_to_char);
        }

        // Drop unknown layer-order entries; warn on names that resolve to no layer.
        let mut layer_order = Vec::new();
        for name in &raw.layer_order {
            if overlay.contains_key(name) {
                layer_order.push(name.clone());
            } else {
                eprintln!("Warning: layer_order entry '{}' has no matching layer; skipping", name);
            }
        }
        for name in overlay.keys() {
            if !layer_order.contains(name) {
                eprintln!("Warning: layer '{}' is not in layer_order; it will be unreachable", name);
            }
        }
        if layer_order.is_empty() {
            return Err("layers config has no usable layers in layer_order".to_string());
        }

        Ok(LayersConfig { layer_order, nav_prev, nav_next, phys_to_logical, overlay, typing })
    }

    /// Character typed for logical key `key` in `layer` (case-insensitive).
    pub fn glyph_for(&self, layer: &str, key: &str) -> Option<char> {
        self.typing.get(layer)?.get(&key.to_uppercase()).copied()
    }
}

/// Title-case a layer name for the default label (`right_arrows` -> `Right Arrows`).
fn derive_label(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build overlay groups for one layer from pre-resolved (logical) global groups and the
/// layer's logical key→glyph map. A key listed in more than one group is assigned to the
/// first. Typeable keys present in the layer but in no group form an unlabeled group
/// rendered first (in `typeable_order`), matching the legacy keyd-derived layout.
fn organize_groups(
    groups: &[(String, Vec<String>)],
    typeable_order: &[String],
    key_to_glyph: &HashMap<String, String>,
) -> Vec<ButtonGroup> {
    let mut assigned: HashSet<String> = HashSet::new();
    let mut named: Vec<ButtonGroup> = Vec::new();

    for (label, keys) in groups {
        let mut buttons = Vec::new();
        for key in keys {
            if !assigned.insert(key.clone()) {
                continue; // a key belongs to its first group only
            }
            if let Some(glyph) = key_to_glyph.get(key) {
                buttons.push(ButtonLegend { display: glyph.clone() });
            }
        }
        if !buttons.is_empty() {
            named.push(ButtonGroup { label: Some(label.clone()), buttons });
        }
    }

    let mut anonymous = Vec::new();
    for key in typeable_order {
        if assigned.contains(key) {
            continue;
        }
        if let Some(glyph) = key_to_glyph.get(key) {
            anonymous.push(ButtonLegend { display: glyph.clone() });
        }
    }

    let mut result = Vec::new();
    if !anonymous.is_empty() {
        result.push(ButtonGroup { label: None, buttons: anonymous });
    }
    result.extend(named);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // [keys] aliases the typeable keys only; F16/F18 are left raw for navigation.
    const SAMPLE: &str = r#"
layer_order = ["main", "symbols"]

[keys]
F13 = "SL1"
F14 = "SL2"
F15 = "SL3"
F17 = "SL4"
F19 = "G1"
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
SL4 = "↚"
G1 = "↜"
G6 = "⇦"

[layers.symbols]
[layers.symbols.buttons]
SL1 = "≠"
G1 = "∈"
"#;

    #[test]
    fn typing_keyed_by_logical_names() {
        let cfg = LayersConfig::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.layer_order, vec!["main", "symbols"]);
        assert_eq!(cfg.glyph_for("main", "SL1"), Some('←'));
        assert_eq!(cfg.glyph_for("main", "G6"), Some('⇦'));
        assert_eq!(cfg.glyph_for("symbols", "G1"), Some('∈'));
        // F-keys are not internal identifiers
        assert_eq!(cfg.glyph_for("main", "F13"), None);
    }

    #[test]
    fn phys_to_logical_is_the_grab_boundary() {
        let cfg = LayersConfig::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.phys_to_logical.get("F13").map(String::as_str), Some("SL1"));
        assert_eq!(cfg.phys_to_logical.get("F24").map(String::as_str), Some("G6"));
        assert_eq!(cfg.phys_to_logical.get("F99"), None);
    }

    #[test]
    fn nav_defaults_to_raw_knob_keys() {
        let cfg = LayersConfig::from_str(SAMPLE).unwrap();
        // [navigation] omitted -> raw F16 / F18 (not aliased).
        assert_eq!(cfg.nav_prev, "F16");
        assert_eq!(cfg.nav_next, "F18");
    }

    #[test]
    fn nav_uses_raw_keycodes() {
        let cfg = LayersConfig::from_str(
            r#"
layer_order = ["x"]
[navigation]
prev = "f17"
next = "F19"
[keys]
F13 = "A"
[layers.x.buttons]
A = "←"
"#,
        )
        .unwrap();
        assert_eq!(cfg.nav_prev, "F17"); // lowercase normalized
        assert_eq!(cfg.nav_next, "F19");
        assert_eq!(cfg.glyph_for("x", "A"), Some('←'));
    }

    #[test]
    fn nav_key_also_aliased_in_keys_is_an_error() {
        // F16 is the (default) nav prev key and is also aliased -> disjointness violated.
        assert!(LayersConfig::from_str(
            r#"
layer_order = ["x"]
[keys]
F13 = "A"
F16 = "KCCW"
[layers.x.buttons]
A = "←"
"#,
        )
        .is_err());
    }

    #[test]
    fn nav_keys_must_differ_and_be_valid_fkeys() {
        // same key for prev and next
        assert!(LayersConfig::from_str(
            r#"
layer_order = ["x"]
[navigation]
prev = "F16"
next = "F16"
[keys]
F13 = "A"
[layers.x.buttons]
A = "←"
"#,
        )
        .is_err());

        // a non-F-key value is rejected (nav takes raw codes, not logical names)
        assert!(LayersConfig::from_str(
            r#"
layer_order = ["x"]
[navigation]
prev = "KCCW"
[keys]
F13 = "A"
[layers.x.buttons]
A = "←"
"#,
        )
        .is_err());
    }

    #[test]
    fn groups_use_logical_names_resolved_to_logical_legends() {
        let cfg = LayersConfig::from_str(SAMPLE).unwrap();
        let main = &cfg.overlay["main"].groups;
        assert_eq!(main.len(), 2);
        assert_eq!(main[0].label, Some("Macropad".to_string()));
        // The first group's keys SL1,SL4 are present -> their glyphs in render order.
        assert_eq!(main[0].buttons.iter().map(|b| b.display.as_str()).collect::<Vec<_>>(), vec!["←", "↚"]);
        assert_eq!(main[1].label, Some("G1-6".to_string()));
        assert_eq!(main[1].buttons.iter().map(|b| b.display.as_str()).collect::<Vec<_>>(), vec!["↜", "⇦"]);
    }

    #[test]
    fn physical_key_reference_in_layer_resolves_to_logical() {
        // A user may reference an aliased physical key directly; it resolves to its logical
        // name internally. (F16/F18 stay raw for navigation.)
        let cfg = LayersConfig::from_str(
            r#"
layer_order = ["x"]
[keys]
F13 = "SL1"
[layers.x.buttons]
F13 = "←"
"#,
        )
        .unwrap();
        assert_eq!(cfg.glyph_for("x", "SL1"), Some('←'));
        // not addressable by physical key past config
        assert_eq!(cfg.glyph_for("x", "F13"), None);
    }

    #[test]
    fn default_label_is_title_cased() {
        let cfg = LayersConfig::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.overlay["main"].label, "Left Arrows");
        assert_eq!(cfg.overlay["symbols"].label, "Symbols");
    }

    #[test]
    fn empty_ring_is_an_error() {
        assert!(LayersConfig::from_str(
            r#"
layer_order = ["ghost"]
[keys]
F13 = "A"
[layers.x.buttons]
A = "z"
"#,
        )
        .is_err());
    }
}
