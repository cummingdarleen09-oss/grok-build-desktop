use std::collections::BTreeMap;

use crate::types::{AuthStyle, ProviderConfig, ProviderFormat, TokenField};

/// A built-in provider the settings UI and the CLI know by id.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub format: ProviderFormat,
    pub base_url: &'static str,
    pub model: &'static str,
    pub env_key: &'static str,
    pub env_key_fallbacks: &'static [&'static str],
    pub context_window: u64,
    pub auth: AuthStyle,
    pub token_field: TokenField,
    pub extra_headers: &'static [(&'static str, &'static str)],
    pub env_http_headers: &'static [(&'static str, &'static str)],
    pub model_hints: &'static [&'static str],
    pub follow_latest: bool,
}

impl Preset {
    pub fn to_config(self, enabled: bool) -> ProviderConfig {
        ProviderConfig {
            id: self.id.to_string(),
            name: self.name.to_string(),
            format: self.format,
            base_url: self.base_url.to_string(),
            model: self.model.to_string(),
            env_key: self.env_key.to_string(),
            env_key_fallbacks: self
                .env_key_fallbacks
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            context_window: self.context_window,
            extra_headers: pairs(self.extra_headers),
            env_http_headers: pairs(self.env_http_headers),
            auth: self.auth,
            enabled,
            temperature: None,
            max_tokens: 8192,
            token_field: self.token_field,
            builtin: true,
            model_hints: self.model_hints.iter().map(|s| (*s).to_string()).collect(),
            follow_latest: self.follow_latest,
        }
    }
}

fn pairs(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Presets offered in the desktop settings and auto-detected from the environment.
///
/// DeepSeek's OpenAI-compatible root is `https://api.deepseek.com` (no `/v1`).
/// Moonshot Kimi's is `https://api.moonshot.cn/v1`.
pub fn builtin_presets() -> &'static [Preset] {
    const PRESETS: &[Preset] = &[
        Preset {
            id: "xai",
            name: "Grok",
            format: ProviderFormat::Responses,
            base_url: "https://api.x.ai/v1",
            model: "grok-4.7",
            env_key: "XAI_API_KEY",
            env_key_fallbacks: &["GROK_CODE_XAI_API_KEY"],
            context_window: 500_000,
            auth: AuthStyle::Bearer,
            token_field: TokenField::MaxTokens,
            extra_headers: &[],
            env_http_headers: &[],
            model_hints: &["grok-4.7", "grok-4.6", "grok-4.5"],
            follow_latest: true,
        },
        Preset {
            id: "deepseek",
            name: "DeepSeek",
            format: ProviderFormat::OpenAi,
            base_url: "https://api.deepseek.com",
            model: "deepseek-chat",
            env_key: "DEEPSEEK_API_KEY",
            env_key_fallbacks: &[],
            context_window: 128_000,
            auth: AuthStyle::Bearer,
            token_field: TokenField::MaxTokens,
            extra_headers: &[],
            env_http_headers: &[],
            model_hints: &["deepseek-chat", "deepseek-reasoner"],
            follow_latest: false,
        },
        Preset {
            id: "kimi",
            name: "Moonshot Kimi",
            format: ProviderFormat::OpenAi,
            base_url: "https://api.moonshot.cn/v1",
            model: "moonshot-v1-128k",
            env_key: "MOONSHOT_API_KEY",
            env_key_fallbacks: &["KIMI_API_KEY"],
            context_window: 128_000,
            auth: AuthStyle::Bearer,
            token_field: TokenField::MaxTokens,
            extra_headers: &[],
            env_http_headers: &[],
            model_hints: &[
                "moonshot-v1-8k",
                "moonshot-v1-32k",
                "moonshot-v1-128k",
                "kimi-latest",
            ],
            follow_latest: false,
        },
        Preset {
            id: "openai",
            name: "OpenAI-compatible",
            format: ProviderFormat::OpenAi,
            base_url: "https://api.openai.com/v1",
            model: "gpt-4o",
            env_key: "OPENAI_API_KEY",
            env_key_fallbacks: &[],
            context_window: 128_000,
            auth: AuthStyle::Bearer,
            token_field: TokenField::MaxCompletionTokens,
            extra_headers: &[],
            env_http_headers: &[],
            model_hints: &["gpt-4o", "gpt-4.1", "o3"],
            follow_latest: false,
        },
        Preset {
            id: "anthropic",
            name: "Anthropic Claude",
            format: ProviderFormat::Anthropic,
            base_url: "https://api.anthropic.com/v1",
            model: "claude-sonnet-4-5",
            env_key: "ANTHROPIC_API_KEY",
            env_key_fallbacks: &[],
            context_window: 200_000,
            auth: AuthStyle::XApiKey,
            token_field: TokenField::MaxTokens,
            extra_headers: &[("anthropic-version", "2023-06-01")],
            env_http_headers: &[("x-api-key", "ANTHROPIC_API_KEY")],
            model_hints: &["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
            follow_latest: false,
        },
    ];
    PRESETS
}

pub fn preset_by_id(id: &str) -> Option<&'static Preset> {
    builtin_presets().iter().find(|preset| preset.id == id)
}

pub fn default_store() -> crate::types::ProviderStore {
    normalize_store(crate::types::ProviderStore {
        active: Some("xai".to_string()),
        providers: builtin_presets()
            .iter()
            .copied()
            .map(|preset| preset.to_config(preset.id == "xai"))
            .collect(),
    })
}

/// Append any built-in preset the file does not mention yet, so upgrades surface
/// new providers without erasing custom ones.
pub fn normalize_store(mut store: crate::types::ProviderStore) -> crate::types::ProviderStore {
    for preset in builtin_presets() {
        if !store
            .providers
            .iter()
            .any(|provider| provider.id == preset.id)
        {
            store.providers.push(preset.to_config(preset.id == "xai"));
        }
    }
    let active_missing = store
        .active
        .as_deref()
        .is_none_or(|id| !store.providers.iter().any(|provider| provider.id == id));
    if active_missing && store.providers.iter().any(|provider| provider.id == "xai") {
        store.active = Some("xai".to_string());
    }
    crate::xai_models::apply_known_models(&mut store);
    store
}
