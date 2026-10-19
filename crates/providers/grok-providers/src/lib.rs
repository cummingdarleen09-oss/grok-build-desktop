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

pub use client::{CompletionRequest, ProviderClient};
pub use config_merge::{
    grok_config_path, grok_home, load_or_default_store, load_provider_store_from,
    merge_env_presets_with, merge_store_presets, providers_path, save_provider_store,
    save_provider_store_to, sync_cli_config, upsert_cli_config,
};
pub use presets::{Preset, builtin_presets, default_store, normalize_store};
pub use secrets::{SecretLocation, SecretStore};
pub use stream::{
    AnthropicAcc, AssistantBuild, OpenAiAcc, SseDecoder, SseEvent, decode_anthropic_event,
    decode_openai_data,
};
pub use translate::{anthropic_request_body, endpoint_url, openai_request_body};
pub use types::*;

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
    match config_merge::load_provider_store_if_present() {
        Ok(Some(store)) => merge_store_presets(raw, &store),
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "ignoring unreadable providers.toml");
        }
    }
    let env = std::env::vars().collect();
    merge_env_presets_with(raw, &env);
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
