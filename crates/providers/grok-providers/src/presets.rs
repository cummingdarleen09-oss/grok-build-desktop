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
            builtin: self.id == "xai",
            model_hints: self.model_hints.iter().map(|s| (*s).to_string()).collect(),
            models: listed(self.model, self.model_hints),
            follow_latest: self.follow_latest,
        }
    }
}

fn listed(model: &str, hints: &[&str]) -> Vec<String> {
    let mut models = Vec::new();
    for id in std::iter::once(model).chain(hints.iter().copied()) {
        let id = id.trim();
        if !id.is_empty() && !models.iter().any(|existing| existing == id) {
            models.push(id.to_string());
        }
    }
    models
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

/// Optional templates. They are not written until the user adds one.
pub fn template_presets() -> Vec<Preset> {
    builtin_presets()
        .iter()
        .copied()
        .filter(|preset| preset.id != "xai")
        .collect()
}

pub fn default_store() -> crate::types::ProviderStore {
    normalize_store(crate::types::ProviderStore {
        active: Some("xai".to_string()),
        providers: vec![preset_by_id("xai").expect("xai preset").to_config(true)],
    })
}

/// Keep Grok built in, turn previously saved vendor rows into custom providers,
/// and fill each model list. Ids and `env_key` stay so keychain entries still match.
pub fn normalize_store(mut store: crate::types::ProviderStore) -> crate::types::ProviderStore {
    if !store.providers.iter().any(|provider| provider.id == "xai") {
        if let Some(preset) = preset_by_id("xai") {
            store.providers.insert(0, preset.to_config(true));
        }
    }
    for provider in &mut store.providers {
        if provider.id == "xai" {
            provider.builtin = true;
        } else {
            provider.builtin = false;
            provider.follow_latest = false;
        }
        fill_model_list(provider);
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

fn fill_model_list(provider: &mut crate::types::ProviderConfig) {
    if provider.models.is_empty() {
        provider.models = listed(
            &provider.model,
            &provider
                .model_hints
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
    }
    if provider.model.trim().is_empty() {
        if let Some(first) = provider.models.first() {
            provider.model = first.clone();
        }
    } else if !provider.models.iter().any(|id| id == &provider.model) {
        provider.models.insert(0, provider.model.clone());
    }
    provider.model_hints = provider.models.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_is_grok_only() {
        let store = default_store();
        assert_eq!(store.providers.len(), 1);
        assert_eq!(store.providers[0].id, "xai");
        assert!(store.providers[0].builtin);
        assert!(store.providers[0].follow_latest);
        assert!(store.providers[0].models.iter().any(|id| id == "grok-4.7"));
    }

    #[test]
    fn saved_vendor_rows_stay_custom_and_keep_the_key_name() {
        let mut deepseek = preset_by_id("deepseek").unwrap().to_config(true);
        deepseek.builtin = true;
        let store = normalize_store(crate::types::ProviderStore {
            active: Some("deepseek".into()),
            providers: vec![deepseek],
        });
        let saved = store
            .providers
            .iter()
            .find(|provider| provider.id == "deepseek")
            .unwrap();
        assert!(!saved.builtin);
        assert_eq!(saved.env_key, "DEEPSEEK_API_KEY");
        assert!(saved.models.iter().any(|id| id == "deepseek-chat"));
        assert!(saved.models.iter().any(|id| id == "deepseek-reasoner"));
        assert!(
            store
                .providers
                .iter()
                .any(|provider| provider.id == "xai" && provider.builtin)
        );
        assert!(store.providers.iter().all(|provider| provider.id != "kimi"));
        assert_eq!(store.active.as_deref(), Some("deepseek"));
    }
}
