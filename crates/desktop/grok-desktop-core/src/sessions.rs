use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use grok_providers::{ReasoningEffort, Role, Turn};

/// Filesystem component limit shared with upstream `encode_cwd_dirname`.
const MAX_DIRNAME_BYTES: usize = 255;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredTool {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    /// `user`, `assistant`, or `compaction`. A compaction message holds the
    /// summary that replaces everything before it in later requests.
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub reasoning: String,
    /// Anthropic's signature over `reasoning`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning_signature: String,
    #[serde(default)]
    pub tool_calls: Vec<StoredTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// Project folder. This is the working directory the agent operates in,
    /// the same key the CLI uses under `~/.grok/sessions/<encoded cwd>`.
    pub cwd: String,
    pub provider_id: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branched_from_message_id: Option<String>,
    /// 1-based index of the chosen message in the parent transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branched_at_index: Option<u32>,
    /// This chat's effort. `None` uses the saved default for the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// This chat's context window in tokens. `None` uses the saved default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: String,
    pub provider_id: String,
    pub cwd: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branched_from_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branched_at_index: Option<u32>,
    /// `desktop` for chats this app wrote, `classic` for upstream TUI sessions.
    #[serde(default = "desktop_origin")]
    pub origin: String,
}

fn desktop_origin() -> String {
    "desktop".into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGroup {
    pub path: String,
    pub sessions: Vec<SessionSummary>,
}

pub fn create_session(cwd: &str, provider_id: &str, model: &str, title: &str) -> Session {
    let now = chrono::Utc::now().to_rfc3339();
    Session {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        cwd: normalize_project_path(cwd),
        provider_id: provider_id.to_string(),
        model: model.to_string(),
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
        parent_session_id: None,
        branched_from_message_id: None,
        branched_at_index: None,
        reasoning_effort: None,
        context_window: None,
    }
}

pub fn save_session(session: &Session) -> Result<(), String> {
    save_session_in(&grok_providers::grok_home(), session)
}

pub fn load_session(id: &str) -> Result<Session, String> {
    load_session_in(&grok_providers::grok_home(), id)
}

/// Read a CLI session as a desktop transcript.
///
/// Text chunks in `updates.jsonl`
/// are shown read-only. Sessions without those chunks still open, with an
/// empty message list, so the sidebar entry is not a dead end.
pub fn load_classic_session(id: &str) -> Result<Session, String> {
    load_classic_session_in(&grok_providers::grok_home(), id)
}

pub fn delete_session(id: &str) -> Result<(), String> {
    delete_session_in(&grok_providers::grok_home(), id)
}

pub fn rename_session(id: &str, title: &str) -> Result<Session, String> {
    let mut session = load_session(id)?;
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("title is empty".into());
    }
    session.title = trimmed.chars().take(80).collect();
    touch(&mut session);
    save_session(&session)?;
    Ok(session)
}

pub fn list_sessions() -> Result<Vec<SessionSummary>, String> {
    list_sessions_in(&grok_providers::grok_home())
}

/// Copy history through `message_id` into a new conversation.
///
/// `include_message` keeps the chosen message. Edit-and-resend passes `false`
/// for a user message so the caller can send the edited text as the next turn.
/// The parent session is not modified.
pub fn branch_session(
    parent_id: &str,
    message_id: &str,
    include_message: bool,
    title: &str,
) -> Result<Session, String> {
    let parent = load_session(parent_id)?;
    let branched = branch_from_session(&parent, message_id, include_message, title)?;
    save_session(&branched)?;
    Ok(branched)
}

pub fn branch_from_session(
    parent: &Session,
    message_id: &str,
    include_message: bool,
    title: &str,
) -> Result<Session, String> {
    let index = parent
        .messages
        .iter()
        .position(|message| message.id == message_id)
        .ok_or_else(|| "message not found".to_string())?;
    if !include_message && parent.messages[index].role != "user" {
        return Err("only a user message can be edited into a new branch".into());
    }
    let end = if include_message { index + 1 } else { index };
    let mut messages = parent.messages[..end].to_vec();
    close_open_tool_round(&mut messages);
    let now = chrono::Utc::now().to_rfc3339();
    let title = {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            format!("Branch · {}", parent.title)
        } else {
            trimmed.to_string()
        }
    };
    Ok(Session {
        id: uuid::Uuid::new_v4().to_string(),
        title,
        cwd: parent.cwd.clone(),
        provider_id: parent.provider_id.clone(),
        model: parent.model.clone(),
        created_at: now.clone(),
        updated_at: now,
        messages,
        parent_session_id: Some(parent.id.clone()),
        branched_from_message_id: Some(message_id.to_string()),
        branched_at_index: Some((index as u32) + 1),
        reasoning_effort: parent.reasoning_effort,
        context_window: parent.context_window,
    })
}

/// Drop an unfinished tool-call round at the end of a copied transcript.
///
/// Tool calls and their results live on the same assistant message. A branch
/// that lands while some calls have no output keeps the finished calls and
/// drops the rest, so the model never sees a tool call without a result. If
/// nothing speakable remains, that assistant message is removed.
pub fn close_open_tool_round(messages: &mut Vec<ChatMessage>) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    if last.role != "assistant" {
        return;
    }
    last.tool_calls.retain(|call| call.output.is_some());
    let empty = last.text.trim().is_empty()
        && last.reasoning.trim().is_empty()
        && last.tool_calls.is_empty();
    if empty {
        messages.pop();
    }
}

pub const COMPACTION_ROLE: &str = "compaction";

/// Provider turns for `messages`, with dangling tool calls omitted. The last
/// compaction message stands in, as a user turn, for everything before it.
pub fn conversation_turns(messages: &[ChatMessage]) -> Vec<Turn> {
    let start = messages
        .iter()
        .rposition(|message| message.role == COMPACTION_ROLE)
        .unwrap_or(0);
    let mut turns = Vec::new();
    for message in &messages[start..] {
        if message.role == "user" || message.role == COMPACTION_ROLE {
            turns.push(Turn::user(&message.text));
            continue;
        }
        if message.role != "assistant" {
            continue;
        }
        let calls: Vec<grok_providers::ToolCall> = message
            .tool_calls
            .iter()
            .filter(|call| call.output.is_some())
            .map(|call| grok_providers::ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            })
            .collect();
        if message.text.trim().is_empty() && message.reasoning.trim().is_empty() && calls.is_empty()
        {
            continue;
        }
        turns.push(Turn {
            role: Role::Assistant,
            text: message.text.clone(),
            reasoning: if message.reasoning.is_empty() {
                None
            } else {
                Some(message.reasoning.clone())
            },
            reasoning_signature: Some(message.reasoning_signature.clone())
                .filter(|signature| !signature.is_empty()),
            tool_calls: calls.clone(),
            tool_call_id: None,
        });
        for call in &message.tool_calls {
            if let Some(output) = &call.output {
                turns.push(Turn::tool(&call.id, output));
            }
        }
    }
    turns
}

pub fn group_sessions(known: &[String], sessions: &[SessionSummary]) -> Vec<ProjectGroup> {
    let mut paths: Vec<String> = known
        .iter()
        .map(|path| normalize_project_path(path))
        .filter(|path| !path.is_empty())
        .collect();
    for session in sessions {
        let path = normalize_project_path(&session.cwd);
        if !path.is_empty() {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let mut grouped: Vec<SessionSummary> = sessions
                .iter()
                .filter(|session| normalize_project_path(&session.cwd) == path)
                .cloned()
                .collect();
            grouped.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            ProjectGroup {
                path,
                sessions: grouped,
            }
        })
        .collect()
}

pub fn normalize_project_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = PathBuf::from(trimmed);
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let mut text = canonical.display().to_string();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        text = stripped.to_string();
    }
    while text.len() > 1 && (text.ends_with('/') || text.ends_with('\\')) {
        text.pop();
    }
    text
}

/// Same directory-name encoding as upstream `xai_grok_config::encode_cwd_dirname`.
pub fn encode_project_dirname(cwd: &str) -> String {
    let url_encoded = urlencoding::encode(cwd);
    if url_encoded.len() <= MAX_DIRNAME_BYTES {
        return url_encoded.into_owned();
    }
    let hash = blake3::hash(cwd.as_bytes());
    let hex = hash.to_hex();
    let leaf = Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let slug = slugify(leaf, 40);
    let slug = if slug.is_empty() { "workspace" } else { &slug };
    let Some(hash16) = hex.get(..16) else {
        return format!("{slug}-{hex}");
    };
    format!("{slug}-{hash16}")
}

pub fn touch(session: &mut Session) {
    session.updated_at = chrono::Utc::now().to_rfc3339();
}

fn save_session_in(home: &Path, session: &Session) -> Result<(), String> {
    validate_id(&session.id)?;
    let dir = project_dir(home, &session.cwd);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    remember_cwd(&dir, &session.cwd)?;
    let path = dir.join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session).map_err(|error| error.to_string())?;
    std::fs::write(&path, json).map_err(|error| error.to_string())?;
    let legacy = desktop_root(home).join(format!("{}.json", session.id));
    if legacy != path && legacy.is_file() {
        let _ = std::fs::remove_file(legacy);
    }
    if let Ok(entries) = std::fs::read_dir(desktop_root(home)) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let other = entry.path().join(format!("{}.json", session.id));
            if other != path && other.is_file() {
                let _ = std::fs::remove_file(other);
            }
        }
    }
    Ok(())
}

fn load_session_in(home: &Path, id: &str) -> Result<Session, String> {
    let path = find_desktop_file(home, id)?;
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn delete_session_in(home: &Path, id: &str) -> Result<(), String> {
    match find_desktop_file(home, id) {
        Ok(path) => std::fs::remove_file(path).map_err(|error| error.to_string()),
        Err(error) if error == "invalid session id" => Err(error),
        Err(_) => Ok(()),
    }
}

fn list_sessions_in(home: &Path) -> Result<Vec<SessionSummary>, String> {
    migrate_flat_sessions(home)?;
    let mut sessions = read_desktop_sessions(home)?;
    sessions.extend(read_classic_sessions(home)?);
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(sessions)
}

fn migrate_flat_sessions(home: &Path) -> Result<(), String> {
    let root = desktop_root(home);
    if !root.exists() {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut session) = serde_json::from_str::<Session>(&text) else {
            continue;
        };
        session.cwd = normalize_project_path(&session.cwd);
        save_session_in(home, &session)?;
    }
    Ok(())
}

/// The fields a sidebar row needs. Message bodies are skipped while parsing,
/// which keeps listing fast when transcripts are large.
#[derive(Deserialize)]
struct SessionHeader {
    id: String,
    title: String,
    cwd: String,
    provider_id: String,
    model: String,
    updated_at: String,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    branched_from_message_id: Option<String>,
    #[serde(default)]
    branched_at_index: Option<u32>,
}

fn read_desktop_sessions(home: &Path) -> Result<Vec<SessionSummary>, String> {
    let root = desktop_root(home);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(nested) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in nested.flatten() {
            let file_path = file.path();
            if file_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file_path) else {
                continue;
            };
            let Ok(header) = serde_json::from_str::<SessionHeader>(&text) else {
                continue;
            };
            sessions.push(SessionSummary {
                id: header.id,
                title: header.title,
                model: header.model,
                provider_id: header.provider_id,
                cwd: header.cwd,
                updated_at: header.updated_at,
                parent_session_id: header.parent_session_id,
                branched_from_message_id: header.branched_from_message_id,
                branched_at_index: header.branched_at_index,
                origin: "desktop".into(),
            });
        }
    }
    Ok(sessions)
}

fn read_classic_sessions(home: &Path) -> Result<Vec<SessionSummary>, String> {
    let root = home.join("sessions");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut sessions = Vec::new();
    let mut normalized: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for entry in entries.flatten() {
        let cwd_dir = entry.path();
        if !cwd_dir.is_dir() {
            continue;
        }
        let decoded = decode_cwd_dir(&cwd_dir);
        let nested = match std::fs::read_dir(&cwd_dir) {
            Ok(nested) => nested,
            Err(_) => continue,
        };
        for session_dir in nested {
            let Ok(session_dir) = session_dir else {
                continue;
            };
            let path = session_dir.path();
            if !path.is_dir() {
                continue;
            }
            let summary_path = path.join("summary.json");
            if !summary_path.is_file() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&summary_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if id.starts_with('.') || id.contains(['/', '\\']) {
                continue;
            }
            let cwd = value
                .pointer("/info/cwd")
                .and_then(|item| item.as_str())
                .filter(|cwd| !cwd.is_empty())
                .map(|cwd| cwd.to_string())
                .or(decoded.clone())
                .unwrap_or_default();
            let title = value
                .get("session_summary")
                .and_then(|item| item.as_str())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or(id)
                .chars()
                .take(80)
                .collect::<String>();
            let model = value
                .get("current_model_id")
                .and_then(|item| item.as_str())
                .unwrap_or("classic")
                .to_string();
            let updated_at = value
                .get("updated_at")
                .and_then(|item| item.as_str())
                .unwrap_or("")
                .to_string();
            sessions.push(SessionSummary {
                id: id.to_string(),
                title,
                model,
                provider_id: "classic".into(),
                cwd: normalized
                    .entry(cwd.clone())
                    .or_insert_with(|| normalize_project_path(&cwd))
                    .clone(),
                updated_at,
                parent_session_id: value
                    .get("parent_session_id")
                    .and_then(|item| item.as_str())
                    .map(|item| item.to_string()),
                branched_from_message_id: None,
                branched_at_index: None,
                origin: "classic".into(),
            });
        }
    }
    Ok(sessions)
}

pub fn load_classic_session_in(home: &Path, id: &str) -> Result<Session, String> {
    validate_id(id)?;
    let root = home.join("sessions");
    let mut found = None;
    if root.is_dir() {
        for cwd_dir in std::fs::read_dir(&root).map_err(|error| error.to_string())? {
            let cwd_dir = cwd_dir.map_err(|error| error.to_string())?;
            let candidate = cwd_dir.path().join(id);
            if candidate.join("summary.json").is_file() {
                found = Some(candidate);
                break;
            }
        }
    }
    let dir = found.ok_or_else(|| format!("classic session {id} was not found"))?;
    let summary_text =
        std::fs::read_to_string(dir.join("summary.json")).map_err(|error| error.to_string())?;
    let summary: serde_json::Value =
        serde_json::from_str(&summary_text).map_err(|error| error.to_string())?;
    let cwd = summary
        .pointer("/info/cwd")
        .and_then(|item| item.as_str())
        .map(normalize_project_path)
        .unwrap_or_default();
    let title = summary
        .get("session_summary")
        .and_then(|item| item.as_str())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(id)
        .chars()
        .take(80)
        .collect::<String>();
    let model = summary
        .get("current_model_id")
        .and_then(|item| item.as_str())
        .unwrap_or("classic")
        .to_string();
    let updated_at = summary
        .get("updated_at")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_string();
    let messages = read_classic_messages(&dir.join("updates.jsonl"));
    Ok(Session {
        id: id.to_string(),
        title,
        cwd,
        provider_id: "classic".into(),
        model,
        created_at: updated_at.clone(),
        updated_at,
        messages,
        parent_session_id: summary
            .get("parent_session_id")
            .and_then(|item| item.as_str())
            .map(str::to_string),
        branched_from_message_id: None,
        branched_at_index: None,
        reasoning_effort: None,
        context_window: None,
    })
}

fn read_classic_messages(path: &Path) -> Vec<ChatMessage> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut messages: Vec<ChatMessage> = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = value
            .get("sessionUpdate")
            .or_else(|| value.pointer("/update/sessionUpdate"))
            .or_else(|| value.pointer("/params/update/sessionUpdate"))
            .and_then(|item| item.as_str())
            .unwrap_or("");
        let role = if kind.contains("user") {
            "user"
        } else if kind.contains("agent") || kind.contains("assistant") {
            "assistant"
        } else {
            continue;
        };
        let text = first_text(&value);
        if text.trim().is_empty() {
            continue;
        }
        if let Some(last) = messages.last_mut() {
            if last.role == role {
                last.text.push('\n');
                last.text.push_str(text.trim());
                continue;
            }
        }
        messages.push(ChatMessage {
            id: format!("classic-{}", messages.len() + 1),
            role: role.into(),
            text: text.trim().to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            reasoning_signature: String::new(),
        });
    }
    messages
}

fn first_text(value: &serde_json::Value) -> String {
    let mut found = String::new();
    collect_text(value, &mut found);
    found
}

fn collect_text(value: &serde_json::Value, found: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|item| item.as_str()) {
                if found.is_empty() {
                    *found = text.to_string();
                    return;
                }
            }
            for (key, child) in map {
                if key == "text" {
                    continue;
                }
                collect_text(child, found);
                if !found.is_empty() {
                    return;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, found);
                if !found.is_empty() {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn desktop_root(home: &Path) -> PathBuf {
    home.join("desktop-sessions")
}

fn project_dir(home: &Path, cwd: &str) -> PathBuf {
    let normalized = normalize_project_path(cwd);
    desktop_root(home).join(encode_project_dirname(&normalized))
}

fn remember_cwd(dir: &Path, cwd: &str) -> Result<(), String> {
    let normalized = normalize_project_path(cwd);
    if encode_project_dirname(&normalized) == urlencoding::encode(&normalized).as_ref() {
        return Ok(());
    }
    std::fs::write(dir.join(".cwd"), &normalized).map_err(|error| error.to_string())
}

fn decode_cwd_dir(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    if let Ok(decoded) = urlencoding::decode(name) {
        let text = decoded.into_owned();
        if text.starts_with('/') || (cfg!(windows) && text.chars().nth(1) == Some(':')) {
            return Some(text);
        }
    }
    std::fs::read_to_string(dir.join(".cwd"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn find_desktop_file(home: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let root = desktop_root(home);
    let legacy = root.join(format!("{id}.json"));
    if legacy.is_file() {
        return Ok(legacy);
    }
    if !root.exists() {
        return Err("missing session".into());
    }
    for entry in std::fs::read_dir(&root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        let candidate = entry.path().join(format!("{id}.json"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("missing session".into())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return Err("invalid session id".into());
    }
    Ok(())
}

fn slugify(input: &str, max_len: usize) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_dash = false;
    for character in input.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            prev_dash = false;
        } else if !prev_dash {
            result.push('-');
            prev_dash = true;
        }
    }
    let trimmed = result.trim_matches('-');
    trimmed.chars().take(max_len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, role: &str, text: &str, tools: Vec<StoredTool>) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            role: role.into(),
            text: text.into(),
            reasoning: String::new(),
            tool_calls: tools,
            reasoning_signature: String::new(),
        }
    }

    fn tool(id: &str, output: Option<&str>) -> StoredTool {
        StoredTool {
            id: id.into(),
            name: "grep".into(),
            arguments: "{}".into(),
            output: output.map(str::to_string),
            is_error: false,
        }
    }

    fn sample_parent() -> Session {
        let mut session = create_session("/workspace/app", "deepseek", "deepseek-chat", "Layout");
        session.id = "parent".into();
        session.messages = vec![
            message("m1", "user", "one", vec![]),
            message("m2", "assistant", "two", vec![tool("t-done", Some("hit"))]),
            message("m3", "user", "three", vec![]),
            message(
                "m4",
                "assistant",
                "",
                vec![tool("t-ok", Some("ok")), tool("t-open", None)],
            ),
            message("m5", "user", "five", vec![]),
            message("m6", "assistant", "six", vec![]),
        ];
        session
    }

    #[test]
    fn project_dirname_matches_upstream_url_encoding() {
        assert_eq!(encode_project_dirname("/workspace"), "%2Fworkspace");
        assert_eq!(
            encode_project_dirname("/home/user/project"),
            "%2Fhome%2Fuser%2Fproject"
        );
    }

    #[test]
    fn storage_groups_desktop_and_classic_sessions_by_project() {
        let home = tempfile::tempdir().unwrap();
        let mut alpha = create_session("/proj/alpha", "deepseek", "deepseek-chat", "Alpha chat");
        alpha.id = "alpha1".into();
        alpha.updated_at = "2026-01-02T00:00:00Z".into();
        let mut beta = create_session("/proj/beta", "kimi", "moonshot", "Beta chat");
        beta.id = "beta1".into();
        beta.updated_at = "2026-01-03T00:00:00Z".into();
        save_session_in(home.path(), &alpha).unwrap();
        save_session_in(home.path(), &beta).unwrap();

        let encoded = encode_project_dirname("/proj/alpha");
        let classic_dir = home.path().join("sessions").join(&encoded).join("classic1");
        std::fs::create_dir_all(&classic_dir).unwrap();
        std::fs::write(
            classic_dir.join("summary.json"),
            r#"{"session_summary":"TUI notes","updated_at":"2026-01-04T00:00:00Z","current_model_id":"grok-4","info":{"cwd":"/proj/alpha"}}"#,
        )
        .unwrap();

        let flat = desktop_root(home.path()).join("legacy1.json");
        std::fs::write(
            &flat,
            serde_json::to_string(&Session {
                id: "legacy1".into(),
                title: "Moved".into(),
                cwd: "/proj/beta".into(),
                provider_id: "openai".into(),
                model: "gpt-4o".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                messages: vec![],
                parent_session_id: None,
                branched_from_message_id: None,
                branched_at_index: None,
                reasoning_effort: None,
                context_window: None,
            })
            .unwrap(),
        )
        .unwrap();

        let listed = list_sessions_in(home.path()).unwrap();
        assert!(!flat.exists());
        let groups = group_sessions(&["/proj/empty".into()], &listed);
        let alpha_group = groups
            .iter()
            .find(|group| group.path.ends_with("alpha"))
            .unwrap();
        let titles: Vec<_> = alpha_group
            .sessions
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert_eq!(titles, vec!["TUI notes", "Alpha chat"]);
        assert_eq!(alpha_group.sessions[0].origin, "classic");
        std::fs::write(
            classic_dir.join("updates.jsonl"),
            "{\"sessionUpdate\":\"user_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"Where is the config?\"}}\n{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"It lives in config.toml.\"}}\n",
        )
        .unwrap();
        let classic = load_classic_session_in(home.path(), "classic1").unwrap();
        assert_eq!(classic.messages.len(), 2);
        assert_eq!(classic.messages[0].role, "user");
        assert!(classic.messages[1].text.contains("config.toml"));
        assert_eq!(classic.provider_id, "classic");
        assert_eq!(alpha_group.sessions[1].origin, "desktop");
        let beta_group = groups
            .iter()
            .find(|group| group.path.ends_with("beta"))
            .unwrap();
        assert_eq!(beta_group.sessions.len(), 2);
        assert!(
            groups
                .iter()
                .any(|group| group.path.ends_with("empty") && group.sessions.is_empty())
        );
        let reloaded = load_session_in(home.path(), "legacy1").unwrap();
        assert_eq!(reloaded.title, "Moved");
        assert!(
            find_desktop_file(home.path(), "alpha1")
                .unwrap()
                .starts_with(desktop_root(home.path()).join(encode_project_dirname("/proj/alpha")))
        );
    }

    #[test]
    fn branch_copies_a_prefix_and_leaves_the_parent_intact() {
        let home = tempfile::tempdir().unwrap();
        let parent = sample_parent();
        save_session_in(home.path(), &parent).unwrap();
        let parent_bytes =
            std::fs::read(find_desktop_file(home.path(), "parent").unwrap()).unwrap();

        let branched = {
            let loaded = load_session_in(home.path(), "parent").unwrap();
            let child = branch_from_session(&loaded, "m3", true, "Branch").unwrap();
            save_session_in(home.path(), &child).unwrap();
            child
        };
        let after = std::fs::read(find_desktop_file(home.path(), "parent").unwrap()).unwrap();
        assert_eq!(parent_bytes, after);
        assert_eq!(
            branched
                .messages
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2", "m3"]
        );
        assert_eq!(branched.cwd, parent.cwd);
        assert_eq!(branched.provider_id, "deepseek");
        assert_eq!(branched.model, "deepseek-chat");
        assert_eq!(branched.parent_session_id.as_deref(), Some("parent"));
        assert_eq!(branched.branched_from_message_id.as_deref(), Some("m3"));
        assert_eq!(branched.branched_at_index, Some(3));

        let turns = conversation_turns(&branched.messages);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].text, "one");
        assert_eq!(turns[1].tool_calls.len(), 1);
        assert_eq!(turns[2].role, Role::Tool);
        assert_eq!(turns[2].tool_call_id.as_deref(), Some("t-done"));
        assert_eq!(turns[3].text, "three");
    }

    #[test]
    fn branch_in_an_open_tool_round_drops_unfinished_calls() {
        let parent = sample_parent();
        let branched = branch_from_session(&parent, "m4", true, "Mid").unwrap();
        assert_eq!(
            branched
                .messages
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2", "m3", "m4"]
        );
        let last = branched.messages.last().unwrap();
        assert_eq!(last.tool_calls.len(), 1);
        assert_eq!(last.tool_calls[0].id, "t-ok");
        assert!(
            parent.messages[3]
                .tool_calls
                .iter()
                .any(|call| call.id == "t-open")
        );

        let turns = conversation_turns(&branched.messages);
        let tool_ids: Vec<_> = turns
            .iter()
            .filter_map(|turn| turn.tool_call_id.clone())
            .collect();
        assert_eq!(tool_ids, vec!["t-done".to_string(), "t-ok".to_string()]);
        assert!(
            turns
                .iter()
                .all(|turn| turn.tool_calls.iter().all(|call| call.id != "t-open"))
        );

        let only_open = branch_from_session(
            &Session {
                messages: vec![
                    message("u", "user", "go", vec![]),
                    message("a", "assistant", "", vec![tool("dangling", None)]),
                ],
                ..parent.clone()
            },
            "a",
            true,
            "Drop",
        )
        .unwrap();
        assert_eq!(only_open.messages.len(), 1);
        assert_eq!(only_open.messages[0].id, "u");
        assert!(
            conversation_turns(&only_open.messages)
                .iter()
                .all(|turn| turn.role == Role::User)
        );
    }

    #[test]
    fn editing_a_user_message_starts_a_branch_without_that_message() {
        let parent = sample_parent();
        let branched = branch_from_session(&parent, "m3", false, "Edited").unwrap();
        assert_eq!(
            branched
                .messages
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(branched.branched_at_index, Some(3));
        assert!(branch_from_session(&parent, "m2", false, "Nope").is_err());
        assert_eq!(parent.messages.len(), 6);
    }
}
