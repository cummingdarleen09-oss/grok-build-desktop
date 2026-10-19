use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use grok_providers::grok_home;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopSettings {
    #[serde(default = "default_lang")]
    pub lang: String,
    /// `ask` requires approval for writes and shell. `auto` runs them.
    #[serde(default = "default_permission")]
    pub permission: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default = "default_active_provider")]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    /// Project folders the user has opened, even when they have no conversations yet.
    #[serde(default)]
    pub known_projects: Vec<String>,
    /// `light` or `dark`. The window starts in the light theme.
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_lang() -> String {
    "zh-CN".into()
}

fn default_permission() -> String {
    "ask".into()
}

fn default_active_provider() -> Option<String> {
    Some("xai".into())
}

fn default_theme() -> String {
    "light".into()
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            lang: default_lang(),
            permission: default_permission(),
            cwd: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display()
                .to_string(),
            active_provider: default_active_provider(),
            disabled_tools: Vec::new(),
            known_projects: Vec::new(),
            theme: default_theme(),
        }
    }
}

impl DesktopSettings {
    pub fn asks_permission(&self) -> bool {
        self.permission != "auto"
    }
}

pub fn settings_path() -> PathBuf {
    grok_home().join("desktop.toml")
}

pub fn load_settings() -> DesktopSettings {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return DesktopSettings::default();
    };
    toml::from_str(&text).unwrap_or_else(|_| DesktopSettings::default())
}

pub fn save_settings(settings: &DesktopSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = toml::to_string_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}
