use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Default, Clone)]
pub struct AppSettings {
    pub keyd_config: Option<String>,
    pub timeout: Option<u64>,
    pub size: Option<String>,
    pub verbose: Option<bool>,
    /// Monitor to display the overlay on. Matched against connector name (e.g. "DP-1"),
    /// then model name, then numeric index. Falls back to the rightmost monitor.
    pub monitor: Option<String>,
    /// Corner to align the overlay to: top-left, top-right, bottom-left, bottom-right.
    pub corner: Option<String>,
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

    fn get_config_path() -> PathBuf {
        let mut path = if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg_config_home)
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config")
        } else {
            PathBuf::from(".")
        };

        path.push("rologlyphex");
        path.push("config.toml");
        path
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_cli(
        &mut self,
        keyd_config: Option<String>,
        timeout: Option<u64>,
        size: Option<String>,
        verbose: Option<bool>,
        monitor: Option<String>,
        corner: Option<String>,
    ) {
        if keyd_config.is_some() {
            self.keyd_config = keyd_config;
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
