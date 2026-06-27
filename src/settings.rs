use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Default, Clone)]
pub struct AppSettings {
    /// Path to the layers glyph-map config (layers.toml). Defaults to
    /// `~/.config/rologlyphex/layers.toml`.
    pub layers: Option<String>,
    pub timeout: Option<u64>,
    pub size: Option<String>,
    pub verbose: Option<bool>,
    /// Monitor to display the overlay on. Matched against connector name (e.g. "DP-1"),
    /// then model name, then numeric index. Falls back to the rightmost monitor.
    pub monitor: Option<String>,
    /// Corner to align the overlay to: top-left, top-right, bottom-left, bottom-right.
    pub corner: Option<String>,
    /// Milliseconds of input quiet after the knob settles on a layer before the typist
    /// remaps for it (the navigation debounce). Default 160. Only used in "debounce" mode.
    pub nav_settle_ms: Option<u64>,
    /// When the typist remaps the keymap for a newly-entered layer: "lazy" (on the first
    /// keypress, with a "Please Wait" overlay) or "debounce" (after the knob settles, no
    /// indicator). Default "lazy".
    pub remap_mode: Option<String>,
    /// Non-BMP (emoji, U+10000+) clipboard-routing settings. See sdd/PLAN.non-BMP.md.
    pub non_bmp: Option<NonBmpSettings>,
}

/// `[non_bmp]` config table.
#[derive(Deserialize, Default, Clone)]
pub struct NonBmpSettings {
    /// WM_CLASS globs whose focused windows receive non-BMP glyphs via the clipboard paste
    /// path instead of the keysym path, which Java/AWT truncates. Matched (anchored glob,
    /// not regex; `*` wildcard) against either res_name or res_class. Empty/unset disables
    /// the clipboard path entirely. Example: `["jetbrains-*"]`.
    pub clipboard_apps: Option<Vec<String>>,
}

impl AppSettings {
    pub fn load() -> Self {
        let config_path = Self::get_config_path();
        if !config_path.exists() {
            return Self::default();
        }

        match fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str::<AppSettings>(&contents) {
                Ok(settings) => settings,
                Err(e) => {
                    eprintln!("Warning: Failed to parse config file at {:?}: {}", config_path, e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Warning: Failed to read config file at {:?}: {}", config_path, e);
                Self::default()
            }
        }
    }

    fn config_dir() -> PathBuf {
        if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config_home)
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config")
        } else {
            PathBuf::from(".")
        }
        .join("rologlyphex")
    }

    fn get_config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Default path to the layers glyph-map config when none is configured.
    pub fn default_layers_path() -> PathBuf {
        Self::config_dir().join("layers.toml")
    }

    #[cfg(test)]
    fn parse(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str::<Self>(s)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_cli(
        &mut self,
        layers: Option<String>,
        timeout: Option<u64>,
        size: Option<String>,
        verbose: Option<bool>,
        monitor: Option<String>,
        corner: Option<String>,
    ) {
        if layers.is_some() {
            self.layers = layers;
        }
        if timeout.is_some() {
            self.timeout = timeout;
        }
        if size.is_some() {
            self.size = size;
        }
        if verbose.is_some() {
            self.verbose = verbose;
        }
        if monitor.is_some() {
            self.monitor = monitor;
        }
        if corner.is_some() {
            self.corner = corner;
        }
    }

}

#[cfg(test)]
mod tests {
    use super::AppSettings;

    #[test]
    fn non_bmp_table_deserializes() {
        let s = AppSettings::parse(
            "corner = \"bottom-left\"\n\n[non_bmp]\nclipboard_apps = [\"jetbrains-*\"]\n",
        )
        .expect("valid config should parse");
        let apps = s.non_bmp.and_then(|n| n.clipboard_apps).unwrap_or_default();
        assert_eq!(apps, vec!["jetbrains-*".to_string()]);
    }

    #[test]
    fn unquoted_corner_value_is_invalid_toml() {
        // Confirms the deployed-config bug: a bare (unquoted) value fails the whole parse,
        // which would silently drop EVERY setting (including [non_bmp]) to defaults.
        assert!(AppSettings::parse("corner = bottom-left\n").is_err());
    }
}
