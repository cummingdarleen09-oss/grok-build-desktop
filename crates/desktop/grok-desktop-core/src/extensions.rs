//! Read-only view of the plugins, skills, and MCP servers the classic grok
//! TUI already loads. Installing and trusting plugins stays in that TUI.

use serde::Serialize;

use grok_providers::grok_home;

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionRecord {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub detail: String,
}

pub fn discover_extensions() -> Vec<ExtensionRecord> {
    let mut records = Vec::new();
    let home = grok_home();
    scan_dir(&home.join("plugins"), "plugin", &mut records);
    scan_dir(&home.join("skills"), "skill", &mut records);
    if let Some(user_home) = dirs_home() {
        scan_dir(
            &user_home.join(".claude").join("plugins"),
            "plugin",
            &mut records,
        );
        scan_dir(
            &user_home.join(".grok").join("skills"),
            "skill",
            &mut records,
        );
    }
    if let Ok(text) = std::fs::read_to_string(home.join("config.toml")) {
        if let Ok(value) = toml::from_str::<toml::Value>(&text) {
            if let Some(servers) = value.get("mcp_servers").and_then(toml::Value::as_table) {
                for (name, spec) in servers {
                    let detail = spec
                        .get("command")
                        .and_then(toml::Value::as_str)
                        .or_else(|| spec.get("url").and_then(toml::Value::as_str))
                        .unwrap_or("mcp server")
                        .to_string();
                    records.push(ExtensionRecord {
                        kind: "mcp".into(),
                        name: name.clone(),
                        path: home.join("config.toml").display().to_string(),
                        detail,
                    });
                }
            }
            if let Some(sources) = value
                .get("marketplace")
                .and_then(|section| section.get("sources"))
                .and_then(toml::Value::as_array)
            {
                for source in sources {
                    let name = source
                        .get("name")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("marketplace")
                        .to_string();
                    let detail = source
                        .get("git")
                        .and_then(toml::Value::as_str)
                        .or_else(|| source.get("path").and_then(toml::Value::as_str))
                        .unwrap_or("")
                        .to_string();
                    records.push(ExtensionRecord {
                        kind: "marketplace".into(),
                        name,
                        path: home.join("config.toml").display().to_string(),
                        detail,
                    });
                }
            }
        }
    }
    let known = home.join("plugins").join("known_marketplaces.json");
    if let Ok(text) = std::fs::read_to_string(&known) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(object) = value.as_object() {
                for name in object.keys() {
                    if records.iter().any(|record| record.name == *name) {
                        continue;
                    }
                    records.push(ExtensionRecord {
                        kind: "marketplace".into(),
                        name: name.clone(),
                        path: known.display().to_string(),
                        detail: "known marketplace".into(),
                    });
                }
            }
        }
    }
    records.sort_by(|left, right| left.kind.cmp(&right.kind).then(left.name.cmp(&right.name)));
    records
}

fn scan_dir(dir: &std::path::Path, kind: &str, records: &mut Vec<ExtensionRecord>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "known_marketplaces.json" {
            continue;
        }
        let detail = if path.is_dir() {
            "directory".to_string()
        } else {
            path.extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("file")
                .to_string()
        };
        records.push(ExtensionRecord {
            kind: kind.to_string(),
            name,
            path: path.display().to_string(),
            detail,
        });
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
}
