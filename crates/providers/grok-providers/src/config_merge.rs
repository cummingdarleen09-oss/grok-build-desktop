//! Fold provider presets into the TOML document the upstream CLI already understands.
//!
//! The shell calls [`crate::merge_configured_providers`] while parsing config.
//! Nothing here writes secrets. Anthropic keys stay in `env_http_headers`
//! (`x-api-key` ← `ANTHROPIC_API_KEY`) because the Messages backend wants that
//! header rather than `Authorization: Bearer`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::presets::{builtin_presets, normalize_store, preset_by_id};
use crate::types::{ProviderConfig, ProviderFormat, ProviderStore};

pub fn grok_home() -> PathBuf {
    if let Some(override_home) = std::env::var_os("GROK_HOME") {
        if !override_home.is_empty() {
            return PathBuf::from(override_home);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
}

pub fn providers_path() -> PathBuf {
    grok_home().join("providers.toml")
}

pub fn grok_config_path() -> PathBuf {
    grok_home().join("config.toml")
}

pub fn load_provider_store_if_present() -> Result<Option<ProviderStore>, String> {
    load_provider_store_from(&providers_path())
}

pub fn load_provider_store_from(path: &Path) -> Result<Option<ProviderStore>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let store: ProviderStore = toml::from_str(&text).map_err(|error| error.to_string())?;
    Ok(Some(normalize_store(store)))
}

pub fn save_provider_store(store: &ProviderStore) -> Result<PathBuf, String> {
    let path = providers_path();
    save_provider_store_to(&path, store)?;
    Ok(path)
}

pub fn save_provider_store_to(path: &Path, store: &ProviderStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = toml::to_string_pretty(store).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

pub fn load_or_default_store() -> ProviderStore {
    match load_provider_store_if_present() {
        Ok(Some(store)) => store,
        Ok(None) => crate::presets::default_store(),
        Err(error) => {
            tracing::warn!(%error, "providers.toml failed to parse; using defaults");
            crate::presets::default_store()
        }
    }
}

/// Insert enabled providers from `providers.toml` when those ids are not already set.
pub fn merge_store_presets(raw: &mut toml::Value, store: &ProviderStore) {
    for provider in store.providers.iter().filter(|provider| provider.enabled) {
        insert_provider_if_absent(raw, provider);
        insert_model_if_absent(raw, &provider.id, provider, &provider.model, &provider.name);
    }
}

/// Insert built-in presets whose API-key environment variable is set.
/// `OPENAI_BASE_URL` / `OPENAI_MODEL` (and the matching vars for the other presets)
/// override the preset defaults. Existing tables win.
pub fn merge_env_presets_with(raw: &mut toml::Value, env: &BTreeMap<String, String>) {
    for preset in builtin_presets() {
        // xAI is merged separately so a `grok login` session counts, and so the
        // model can follow the cached newest id without an HTTP call.
        if preset.id == "xai" {
            continue;
        }
        let key_set = std::iter::once(preset.env_key)
            .chain(preset.env_key_fallbacks.iter().copied())
            .any(|name| env_nonempty(env, name).is_some());
        if !key_set {
            continue;
        }
        let mut provider = preset.to_config(true);
        let base_var = format!("{}_BASE_URL", preset.env_key.trim_end_matches("_API_KEY"));
        // DEEPSEEK_API_KEY → DEEPSEEK_BASE_URL, MOONSHOT_API_KEY → MOONSHOT_BASE_URL, etc.
        if let Some(base_url) = env_nonempty(env, &base_var) {
            provider.base_url = base_url;
        }
        let model_var = format!("{}_MODEL", preset.env_key.trim_end_matches("_API_KEY"));
        if let Some(model) = env_nonempty(env, &model_var) {
            provider.model = model;
        }
        insert_provider_if_absent(raw, &provider);
        if preset.id == "deepseek" && env_nonempty(env, "DEEPSEEK_MODEL").is_none() {
            insert_model_if_absent(
                raw,
                "deepseek-chat",
                &provider,
                "deepseek-chat",
                "DeepSeek Chat",
            );
            insert_model_if_absent(
                raw,
                "deepseek-reasoner",
                &provider,
                "deepseek-reasoner",
                "DeepSeek Reasoner",
            );
        } else {
            let key = if preset.id == "anthropic" {
                "claude"
            } else {
                preset.id
            };
            insert_model_if_absent(raw, key, &provider, &provider.model, &provider.name);
        }
    }
}

/// Insert the xAI preset when a key or a `grok login` session exists.
///
/// Does not call the network. The model is `XAI_MODEL` when that variable is
/// set (a pin), otherwise the cached `/v1/models` choice or the offline
/// fallback. An existing `[model_providers.xai]` or `[model.xai]` table wins.
pub fn merge_xai_preset(
    raw: &mut toml::Value,
    env: &BTreeMap<String, String>,
    login: &crate::xai_auth::LoginSnapshot,
) {
    let has_credential = crate::xai_auth::env_key(env).is_some()
        || login.session.is_some()
        || login.api_key.is_some();
    if !has_credential {
        return;
    }
    let Some(preset) = preset_by_id("xai") else {
        return;
    };
    let mut provider = preset.to_config(true);
    if let Some(base_url) = env_nonempty(env, "XAI_BASE_URL") {
        provider.base_url = base_url;
    }
    if let Some(model) = env_nonempty(env, "XAI_MODEL") {
        provider.model = model;
        provider.follow_latest = false;
    } else {
        let choice = crate::xai_models::choice_from_cache_or_offline();
        provider.model = choice.latest;
        provider.model_hints = choice.models;
    }
    insert_provider_if_absent(raw, &provider);
    insert_model_if_absent(raw, "xai", &provider, &provider.model, &provider.name);
}

fn env_nonempty(env: &BTreeMap<String, String>, name: &str) -> Option<String> {
    env.get(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn insert_provider_if_absent(raw: &mut toml::Value, provider: &ProviderConfig) {
    let Some(root) = raw.as_table_mut() else {
        return;
    };
    let Some(providers) = ensure_table(root, "model_providers") else {
        return;
    };
    if providers.contains_key(&provider.id) {
        return;
    }
    providers.insert(
        provider.id.clone(),
        toml::Value::Table(provider_table(provider)),
    );
}

fn insert_model_if_absent(
    raw: &mut toml::Value,
    key: &str,
    provider: &ProviderConfig,
    model: &str,
    name: &str,
) {
    let Some(root) = raw.as_table_mut() else {
        return;
    };
    let Some(models) = ensure_table(root, "model") else {
        return;
    };
    if models.contains_key(key) {
        return;
    }
    let mut table = toml::map::Map::new();
    table.insert("model".into(), toml::Value::String(model.to_string()));
    table.insert("name".into(), toml::Value::String(name.to_string()));
    table.insert(
        "description".into(),
        toml::Value::String(format!(
            "{name} via the grok-desktop {} provider preset",
            provider.format.as_str()
        )),
    );
    table.insert(
        "model_provider".into(),
        toml::Value::String(provider.id.clone()),
    );
    table.insert(
        "context_window".into(),
        toml::Value::Integer(provider.context_window.min(i64::MAX as u64) as i64),
    );
    models.insert(key.to_string(), toml::Value::Table(table));
}

fn provider_table(provider: &ProviderConfig) -> toml::map::Map<String, toml::Value> {
    let mut table = toml::map::Map::new();
    table.insert(
        "base_url".into(),
        toml::Value::String(provider.base_url.clone()),
    );
    table.insert(
        "api_backend".into(),
        toml::Value::String(provider.format.api_backend().to_string()),
    );
    if provider.format == ProviderFormat::OpenAi && !provider.env_key.is_empty() {
        if provider.env_key_fallbacks.is_empty() {
            table.insert(
                "env_key".into(),
                toml::Value::String(provider.env_key.clone()),
            );
        } else {
            let mut keys = vec![toml::Value::String(provider.env_key.clone())];
            keys.extend(
                provider
                    .env_key_fallbacks
                    .iter()
                    .cloned()
                    .map(toml::Value::String),
            );
            table.insert("env_key".into(), toml::Value::Array(keys));
        }
    }
    if !provider.extra_headers.is_empty() {
        table.insert(
            "extra_headers".into(),
            toml::Value::Table(string_table(&provider.extra_headers)),
        );
    }
    if !provider.env_http_headers.is_empty() {
        table.insert(
            "env_http_headers".into(),
            toml::Value::Table(string_table(&provider.env_http_headers)),
        );
    }
    let _ = preset_by_id(&provider.id);
    table
}

fn string_table(map: &BTreeMap<String, String>) -> toml::map::Map<String, toml::Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), toml::Value::String(value.clone())))
        .collect()
}

fn ensure_table<'a>(
    parent: &'a mut toml::map::Map<String, toml::Value>,
    key: &str,
) -> Option<&'a mut toml::map::Map<String, toml::Value>> {
    match parent.get(key) {
        Some(toml::Value::Table(_)) => {}
        Some(_) => return None,
        None => {
            parent.insert(key.to_string(), toml::Value::Table(toml::map::Map::new()));
        }
    }
    match parent.get_mut(key) {
        Some(toml::Value::Table(table)) => Some(table),
        _ => None,
    }
}

/// Write enabled providers into an existing grok `config.toml` document.
/// User sections with the same id are updated in place. Secrets are not written.
pub fn upsert_cli_config(document: &mut toml_edit::DocumentMut, providers: &[ProviderConfig]) {
    for provider in providers.iter().filter(|provider| provider.enabled) {
        upsert_provider_table(document, provider);
        upsert_model_table(document, provider);
    }
}

fn upsert_provider_table(document: &mut toml_edit::DocumentMut, provider: &ProviderConfig) {
    let providers = ensure_edit_table(document, "model_providers");
    let table = ensure_edit_subtable(providers, &provider.id);
    set_str(table, "base_url", &provider.base_url);
    set_str(table, "api_backend", provider.format.api_backend());
    if provider.format == ProviderFormat::OpenAi && !provider.env_key.is_empty() {
        if provider.env_key_fallbacks.is_empty() {
            set_str(table, "env_key", &provider.env_key);
        } else {
            let mut array = toml_edit::Array::new();
            array.push(provider.env_key.as_str());
            for fallback in &provider.env_key_fallbacks {
                array.push(fallback.as_str());
            }
            table.insert(
                "env_key",
                toml_edit::Item::Value(toml_edit::Value::Array(array)),
            );
        }
    } else {
        table.remove("env_key");
    }
    if provider.extra_headers.is_empty() {
        table.remove("extra_headers");
    } else {
        table.insert(
            "extra_headers",
            toml_edit::Item::Value(inline_strings(&provider.extra_headers)),
        );
    }
    if provider.env_http_headers.is_empty() {
        table.remove("env_http_headers");
    } else {
        table.insert(
            "env_http_headers",
            toml_edit::Item::Value(inline_strings(&provider.env_http_headers)),
        );
    }
}

fn upsert_model_table(document: &mut toml_edit::DocumentMut, provider: &ProviderConfig) {
    let models = ensure_edit_table(document, "model");
    let table = ensure_edit_subtable(models, &provider.id);
    set_str(table, "model", &provider.model);
    set_str(table, "name", &provider.name);
    set_str(
        table,
        "description",
        &format!(
            "{} via the grok-desktop {} provider preset",
            provider.name,
            provider.format.as_str()
        ),
    );
    set_str(table, "model_provider", &provider.id);
    table.insert(
        "context_window",
        toml_edit::Item::Value(toml_edit::Value::from(provider.context_window as i64)),
    );
}

fn ensure_edit_table<'a>(
    document: &'a mut toml_edit::DocumentMut,
    key: &str,
) -> &'a mut toml_edit::Table {
    if !document.contains_key(key) {
        document.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    document
        .get_mut(key)
        .and_then(toml_edit::Item::as_table_mut)
        .expect("config section is a table")
}

fn ensure_edit_subtable<'a>(
    parent: &'a mut toml_edit::Table,
    key: &str,
) -> &'a mut toml_edit::Table {
    if parent
        .get(key)
        .and_then(toml_edit::Item::as_table)
        .is_none()
    {
        parent.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(toml_edit::Item::as_table_mut)
        .expect("nested config section is a table")
}

fn set_str(table: &mut toml_edit::Table, key: &str, value: &str) {
    table.insert(key, toml_edit::Item::Value(toml_edit::Value::from(value)));
}

fn inline_strings(map: &BTreeMap<String, String>) -> toml_edit::Value {
    let mut inline = toml_edit::InlineTable::new();
    for (key, value) in map {
        inline.insert(key, toml_edit::Value::from(value.as_str()));
    }
    toml_edit::Value::InlineTable(inline)
}

/// Read `config.toml`, upsert enabled providers, and write it back.
pub fn sync_cli_config(providers: &[ProviderConfig]) -> Result<PathBuf, String> {
    let path = grok_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut document = existing
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());
    upsert_cli_config(&mut document, providers);
    std::fs::write(&path, document.to_string()).map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::preset_by_id;

    #[test]
    fn env_preset_does_not_clobber_user_model() {
        let mut raw: toml::Value = toml::from_str(
            r#"
            [model_providers.deepseek]
            base_url = "https://private.example/v1"

            [model.deepseek-chat]
            model = "my-local"
            model_provider = "deepseek"
            context_window = 1000
            "#,
        )
        .unwrap();
        let mut env = BTreeMap::new();
        env.insert("DEEPSEEK_API_KEY".into(), "sk-test".into());
        merge_env_presets_with(&mut raw, &env);
        let provider = raw.get("model_providers").unwrap().get("deepseek").unwrap();
        assert_eq!(
            provider.get("base_url").and_then(toml::Value::as_str),
            Some("https://private.example/v1")
        );
        assert_eq!(
            raw.get("model")
                .unwrap()
                .get("deepseek-chat")
                .unwrap()
                .get("model")
                .and_then(toml::Value::as_str),
            Some("my-local")
        );
        assert!(raw.get("model").unwrap().get("deepseek-reasoner").is_some());
    }

    #[test]
    fn env_preset_builds_kimi_and_claude_shapes() {
        let mut raw = toml::Value::Table(toml::map::Map::new());
        let mut env = BTreeMap::new();
        env.insert("MOONSHOT_API_KEY".into(), "sk-moon".into());
        env.insert("ANTHROPIC_API_KEY".into(), "sk-ant".into());
        env.insert("ANTHROPIC_MODEL".into(), "claude-opus-4-5".into());
        env.insert("OPENAI_BASE_URL".into(), "https://proxy.example/v1".into());
        merge_env_presets_with(&mut raw, &env);
        let kimi = raw.get("model_providers").unwrap().get("kimi").unwrap();
        assert_eq!(
            kimi.get("base_url").and_then(toml::Value::as_str),
            Some("https://api.moonshot.cn/v1")
        );
        assert!(kimi.get("env_key").unwrap().as_array().is_some());
        let claude = raw.get("model").unwrap().get("claude").unwrap();
        assert_eq!(
            claude.get("model").and_then(toml::Value::as_str),
            Some("claude-opus-4-5")
        );
        let anthropic = raw
            .get("model_providers")
            .unwrap()
            .get("anthropic")
            .unwrap();
        assert_eq!(
            anthropic.get("api_backend").and_then(toml::Value::as_str),
            Some("messages")
        );
        assert!(anthropic.get("env_key").is_none());
        assert_eq!(
            anthropic
                .get("env_http_headers")
                .unwrap()
                .get("x-api-key")
                .and_then(toml::Value::as_str),
            Some("ANTHROPIC_API_KEY")
        );
        assert!(raw.get("model_providers").unwrap().get("openai").is_none());
    }

    #[test]
    fn store_merge_skips_disabled_providers() {
        let mut store = crate::presets::default_store();
        store
            .providers
            .push(preset_by_id("deepseek").unwrap().to_config(true));
        let mut raw = toml::Value::Table(toml::map::Map::new());
        merge_store_presets(&mut raw, &store);
        let providers = raw.get("model_providers").unwrap();
        assert!(providers.get("xai").is_some());
        assert!(providers.get("deepseek").is_some());
        assert!(providers.get("kimi").is_none());
        assert_eq!(
            providers
                .get("xai")
                .unwrap()
                .get("api_backend")
                .and_then(toml::Value::as_str),
            Some("responses")
        );
        assert!(providers.get("xai").unwrap().get("env_key").is_none());
    }

    #[test]
    fn xai_env_preset_follows_latest_until_xai_model_pins_it() {
        let mut raw = toml::Value::Table(toml::map::Map::new());
        let login = crate::xai_auth::LoginSnapshot {
            session: Some("session-token".into()),
            api_key: None,
        };
        merge_xai_preset(&mut raw, &BTreeMap::new(), &login);
        let model = raw
            .get("model")
            .unwrap()
            .get("xai")
            .unwrap()
            .get("model")
            .and_then(toml::Value::as_str)
            .unwrap();
        assert!(model.starts_with("grok-"), "{model}");
        assert_eq!(
            raw.get("model_providers")
                .unwrap()
                .get("xai")
                .unwrap()
                .get("api_backend")
                .and_then(toml::Value::as_str),
            Some("responses")
        );

        let mut pinned = toml::Value::Table(toml::map::Map::new());
        let mut env = BTreeMap::new();
        env.insert("XAI_API_KEY".into(), "sk-test".into());
        env.insert("XAI_MODEL".into(), "grok-4.5".into());
        env.insert("XAI_BASE_URL".into(), "https://us.api.x.ai/v1".into());
        merge_xai_preset(
            &mut pinned,
            &env,
            &crate::xai_auth::LoginSnapshot::default(),
        );
        assert_eq!(
            pinned
                .get("model")
                .unwrap()
                .get("xai")
                .unwrap()
                .get("model")
                .and_then(toml::Value::as_str),
            Some("grok-4.5")
        );
        assert_eq!(
            pinned
                .get("model_providers")
                .unwrap()
                .get("xai")
                .unwrap()
                .get("base_url")
                .and_then(toml::Value::as_str),
            Some("https://us.api.x.ai/v1")
        );
    }

    #[test]
    fn upsert_roundtrip_preserves_unrelated_keys() {
        let mut document = r#"
            theme = "dark"

            [model.grok]
            model = "grok-4"
            context_window = 128000
        "#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
        let mut provider = preset_by_id("deepseek").unwrap().to_config(true);
        provider.model = "deepseek-chat".into();
        upsert_cli_config(&mut document, &[provider]);
        let text = document.to_string();
        assert!(text.contains("theme = \"dark\""));
        assert!(text.contains("model = \"grok-4\""));
        assert!(text.contains("https://api.deepseek.com"));
        assert!(text.contains("api_backend = \"chat_completions\""));
        assert!(!text.contains("sk-"));
    }
}
