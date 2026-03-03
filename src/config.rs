use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub max_visible: Option<usize>,
    #[serde(default)]
    pub specs_dir: Option<PathBuf>,
    /// Show a description panel to the right of the popup for the selected item.
    #[serde(default)]
    pub show_description_panel: Option<bool>,
}

impl Config {
    /// Load config from ~/.config/melon/config.toml (if it exists).
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("melon")
            .join("config.toml")
    }

    pub fn specs_dir(&self) -> PathBuf {
        self.specs_dir.clone().unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("melon")
                .join("specs")
        })
    }
}
