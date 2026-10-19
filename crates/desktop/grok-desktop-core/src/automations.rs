//! Scheduled prompts for the desktop workbench.
//!
//! The shape follows the host-side automation record in RongleCat/grok-app
//! (MIT). This app runs a due task only while the window is open, by creating
//! a desktop chat and sending the prompt through the selected provider.

use serde::{Deserialize, Serialize};

use grok_providers::grok_home;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Automation {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    /// `daily`, `weekly`, `weekdays`, or `once`.
    #[serde(default = "default_frequency")]
    pub frequency: String,
    /// Local time `HH:MM`.
    #[serde(default = "default_time")]
    pub time: String,
    #[serde(default)]
    pub weekdays: Vec<u8>,
    #[serde(default)]
    pub last_run_at: String,
    #[serde(default)]
    pub next_run_at: String,
}

fn default_true() -> bool {
    true
}
fn default_frequency() -> String {
    "daily".into()
}
fn default_time() -> String {
    "09:00".into()
}

fn path() -> std::path::PathBuf {
    grok_home().join("desktop-automations.json")
}

pub fn load_automations() -> Vec<Automation> {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_automations(list: &[Automation]) -> Result<(), String> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_record_round_trips() {
        let item = Automation {
            id: "a1".into(),
            title: "Morning plan".into(),
            prompt: "Summarize yesterday's diff.".into(),
            enabled: true,
            cwd: "/work/app".into(),
            provider_id: "xai".into(),
            model: "grok-4.7".into(),
            frequency: "weekdays".into(),
            time: "09:30".into(),
            weekdays: vec![1, 2, 3, 4, 5],
            last_run_at: String::new(),
            next_run_at: String::new(),
        };
        let text = serde_json::to_string(&item).unwrap();
        let back: Automation = serde_json::from_str(&text).unwrap();
        assert_eq!(back, item);
    }
}
