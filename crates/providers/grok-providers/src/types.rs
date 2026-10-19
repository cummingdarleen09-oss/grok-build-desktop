use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderFormat {
    /// OpenAI Chat Completions, including DeepSeek and Moonshot Kimi.
    #[serde(rename = "openai")]
    OpenAi,
    /// Anthropic Messages API (`/v1/messages`).
    #[serde(rename = "anthropic")]
    Anthropic,
    /// OpenAI Responses API (`/v1/responses`) for any base URL. xAI Grok uses this.
    #[serde(rename = "responses")]
    Responses,
}

impl ProviderFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Responses => "responses",
        }
    }

    pub fn api_backend(self) -> &'static str {
        match self {
            Self::OpenAi => "chat_completions",
            Self::Anthropic => "messages",
            Self::Responses => "responses",
        }
    }

    pub fn path_suffix(self) -> &'static str {
        match self {
            Self::OpenAi => "chat/completions",
            Self::Anthropic => "messages",
            Self::Responses => "responses",
        }
    }
}

/// How the API key is attached to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthStyle {
    #[serde(rename = "bearer")]
    Bearer,
    /// Anthropic's `x-api-key` header.
    #[serde(rename = "x_api_key")]
    XApiKey,
}

impl Default for AuthStyle {
    fn default() -> Self {
        Self::Bearer
    }
}

/// Which token-limit field the chat body uses.
///
/// DeepSeek and Kimi expect `max_tokens`. Newer OpenAI models expect
/// `max_completion_tokens` and reject the older name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenField {
    #[serde(rename = "max_tokens")]
    MaxTokens,
    #[serde(rename = "max_completion_tokens")]
    MaxCompletionTokens,
}

impl Default for TokenField {
    fn default() -> Self {
        Self::MaxTokens
    }
}

fn default_max_tokens() -> u32 {
    8192
}

/// One configured provider. Secrets are not stored here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub format: ProviderFormat,
    pub base_url: String,
    pub model: String,
    /// Environment variable the CLI should read. The desktop app keeps the
    /// secret in the keychain or a private file, not in this struct.
    pub env_key: String,
    #[serde(default)]
    pub env_key_fallbacks: Vec<String>,
    pub context_window: u64,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// Header name → environment variable name. Written through to grok config
    /// so the CLI can attach `x-api-key` without putting the secret in the file.
    #[serde(default)]
    pub env_http_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub auth: AuthStyle,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub token_field: TokenField,
    /// True for DeepSeek, Kimi, OpenAI, Anthropic, and xAI presets.
    #[serde(default)]
    pub builtin: bool,
    /// Model ids the settings UI offers as suggestions. Kept in sync with `models`.
    #[serde(default)]
    pub model_hints: Vec<String>,
    /// Every model this provider exposes in the composer. `model` is the default.
    #[serde(default)]
    pub models: Vec<String>,
    /// When true, `model` tracks the newest general Grok model. A settings pin sets this false.
    #[serde(default)]
    pub follow_latest: bool,
}

impl ProviderConfig {
    pub fn env_names(&self) -> Vec<&str> {
        let mut names = Vec::with_capacity(1 + self.env_key_fallbacks.len());
        if !self.env_key.trim().is_empty() {
            names.push(self.env_key.as_str());
        }
        for name in &self.env_key_fallbacks {
            if !name.trim().is_empty() {
                names.push(name.as_str());
            }
        }
        names
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(
                "provider id must be letters, numbers, '-' or '_' and cannot be empty".into(),
            );
        }
        if self.model.trim().is_empty() && self.models.iter().all(|model| model.trim().is_empty()) {
            return Err("at least one model is required".into());
        }
        if !(self.base_url.starts_with("https://") || self.base_url.starts_with("http://")) {
            return Err("base URL must start with https:// or http://".into());
        }
        if self.context_window == 0 {
            return Err("context window must be greater than zero".into());
        }
        if self.max_tokens == 0 {
            return Err("max tokens must be greater than zero".into());
        }
        Ok(())
    }
}

/// `providers.toml` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProviderStore {
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One turn in the provider-neutral transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    #[serde(default)]
    pub text: String,
    /// DeepSeek reasoner requires this to be echoed back on later requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Turn {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            text: text.into(),
            reasoning: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            reasoning: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn tool(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            text: text.into(),
            reasoning: None,
            tool_calls: Vec::new(),
            tool_call_id: Some(id.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON object text.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    ToolUse,
    Length,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEvent {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolStart {
        index: u32,
        id: String,
        name: String,
    },
    ToolArgs {
        index: u32,
        fragment: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Stop {
        reason: StopReason,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider {0} has no API key")]
    MissingKey(String),
    #[error("{0}")]
    Invalid(String),
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn redact_secret(body: &str, secret: &str) -> String {
    if secret.len() < 4 {
        return body.to_string();
    }
    body.replace(secret, "•••")
}
