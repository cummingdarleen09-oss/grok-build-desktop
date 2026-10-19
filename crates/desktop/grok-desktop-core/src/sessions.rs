use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use grok_providers::grok_home;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTool {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    /// `user` or `assistant`.
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tool_calls: Vec<StoredTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub provider_id: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: String,
    pub provider_id: String,
    pub updated_at: String,
}

pub fn sessions_dir() -> PathBuf {
    grok_home().join("desktop-sessions")
}

pub fn create_session(cwd: &str, provider_id: &str, model: &str, title: &str) -> Session {
    let now = chrono::Utc::now().to_rfc3339();
    Session {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        cwd: cwd.to_string(),
        provider_id: provider_id.to_string(),
        model: model.to_string(),
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
    }
}

pub fn save_session(session: &Session) -> Result<(), String> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

pub fn load_session(id: &str) -> Result<Session, String> {
    if id.contains(['/', '\\', '.']) {
        return Err("invalid session id".into());
    }
    let path = sessions_dir().join(format!("{id}.json"));
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

pub fn delete_session(id: &str) -> Result<(), String> {
    if id.contains(['/', '\\', '.']) {
        return Err("invalid session id".into());
    }
    let path = sessions_dir().join(format!("{id}.json"));
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn list_sessions() -> Result<Vec<SessionSummary>, String> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_str::<Session>(&text) else {
            continue;
        };
        sessions.push(SessionSummary {
            id: session.id,
            title: session.title,
            model: session.model,
            provider_id: session.provider_id,
            updated_at: session.updated_at,
        });
    }
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(sessions)
}

pub fn touch(session: &mut Session) {
    session.updated_at = chrono::Utc::now().to_rfc3339();
}
