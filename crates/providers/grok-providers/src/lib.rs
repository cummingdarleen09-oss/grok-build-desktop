//! Pluggable chat providers for the grok-build desktop fork.
//!
//! The upstream sampler already speaks OpenAI Chat Completions and Anthropic
//! Messages. This crate is the small, tested front door for that: preset
//! definitions (DeepSeek, Moonshot Kimi, generic OpenAI, generic Claude),
//! request translation, SSE parsing, secret storage, and the config snippets
//! the existing CLI reads from `config.toml` / environment variables.
//!
//! Upstream: <https://github.com/xai-org/grok-build> (Apache-2.0).

#![forbid(unsafe_code)]

mod client;
mod config_merge;
mod presets;
mod secrets;
mod stream;
mod translate;
mod types;
mod xai_auth;
mod xai_models;

pub use client::{CompletionRequest, ProviderClient};
pub use config_merge::{
    grok_config_path, grok_home, load_or_default_store, load_provider_store_from,
    merge_env_presets_with, merge_store_presets, providers_path, save_provider_store,
    save_provider_store_to, sync_cli_config, upsert_cli_config,
};
pub use presets::{Preset, builtin_presets, default_store, normalize_store};
pub use secrets::{SecretLocation, SecretStore};
pub use stream::{
    AnthropicAcc, AssistantBuild, OpenAiAcc, ResponsesAcc, SseDecoder, SseEvent,
    decode_anthropic_event, decode_openai_data, decode_responses_data,
};
pub use translate::{anthropic_request_body, endpoint_url, openai_request_body};
pub use types::*;
pub use xai_auth::{has_login_session, pick_xai_secret};
pub use xai_models::{apply_choice, refresh_xai_models, refresh_xai_models_blocking};

/// Read `~/.grok/providers.toml` (or `$GROK_HOME/providers.toml`) and the
/// process environment, and fold any configured third-party providers into a
/// grok `config.toml` value. Existing user keys are left alone.
///
/// Called from `xai-grok-shell` while loading config. A bad providers file is
/// ignored so a desktop settings typo cannot stop the CLI from starting.
pub fn merge_configured_providers(raw: &mut toml::Value) {
    if presets_disabled() {
        return;
    }
    let mut loaded = None;
    match config_merge::load_provider_store_if_present() {
        Ok(Some(store)) => {
            merge_store_presets(raw, &store);
            loaded = Some(store);
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "ignoring unreadable providers.toml");
        }
    }
    let env = std::env::vars().collect();
    merge_env_presets_with(raw, &env);
    let xai_disabled = loaded.as_ref().is_some_and(|store| {
        store
            .providers
            .iter()
            .any(|provider| provider.id == "xai" && !provider.enabled)
    });
    if !xai_disabled {
        config_merge::merge_xai_preset(raw, &env, &xai_auth::current_login());
    }
}

fn presets_disabled() -> bool {
    matches!(
        std::env::var("GROK_DISABLE_PROVIDER_PRESETS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}
