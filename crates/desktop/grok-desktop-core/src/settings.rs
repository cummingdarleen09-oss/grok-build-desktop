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
            cwd: default_project_dir(),
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

/// The folder a new install works in. An app started from Finder, the Start
/// menu, or a desktop shortcut has `/` or `C:\Windows\System32` as its process
/// directory, which is no place for an agent to read and write files.
pub fn default_project_dir() -> String {
    let current = std::env::current_dir().ok();
    let usable = current.filter(|dir| !is_system_dir(dir));
    usable
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .display()
        .to_string()
}

fn is_system_dir(dir: &std::path::Path) -> bool {
    if dir.parent().is_none() {
        return true;
    }
    let text = dir
        .display()
        .to_string()
        .to_ascii_lowercase()
        .replace('/', "\\");
    text.contains("\\windows\\system32")
        || text.contains("\\program files")
        || text.ends_with(".app\\contents\\macos")
        || dirs::home_dir().is_some_and(|home| home.parent() == Some(dir))
}

pub fn load_settings() -> DesktopSettings {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return DesktopSettings::default();
    };
    let mut settings: DesktopSettings =
        toml::from_str(&text).unwrap_or_else(|_| DesktopSettings::default());
    if settings.cwd.trim().is_empty() || is_system_dir(std::path::Path::new(settings.cwd.trim())) {
        settings.cwd = default_project_dir();
    }
    settings
}

pub fn save_settings(settings: &DesktopSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = toml::to_string_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_folders_of_a_gui_app_are_not_used_as_the_project() {
        assert!(is_system_dir(std::path::Path::new("/")));
        assert!(is_system_dir(std::path::Path::new(r"C:\Windows\System32")));
        assert!(is_system_dir(std::path::Path::new(
            "/Applications/Grok Desktop.app/Contents/MacOS"
        )));
        assert!(!is_system_dir(std::path::Path::new("/home/ada/code/app")));
        assert!(!default_project_dir().is_empty());
    }
}
