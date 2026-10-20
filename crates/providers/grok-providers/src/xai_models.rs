//! Which Grok models the xAI preset offers, and which one it calls by default.
//!
//! The list follows the CLI (`xai-grok-shell` `remote::model_source::oai`):
//! when `GET /v1/models` succeeds, every entry it returns is offered, in the
//! server's order, with the server's display name. Fast variants are separate
//! entries and are not filtered. Only `hidden` entries, and `supportedInApi:
//! false` entries for API-key users, are left out, as in the CLI picker.
//! Otherwise the list is the cached response, then the built-in catalog.
//!
//! A Grok sign-in session lists models through the CLI chat proxy with the
//! CLI's headers. An API key lists them from `{base_url}/models`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config_merge::grok_home;
use crate::models::{ModelInfo, known_models, parse_model_entries};
use crate::types::ProviderStore;

/// How long a `/v1/models` response stays fresh for the CLI.
/// The desktop app refreshes on every launch and writes this cache.
pub const MODEL_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Confirmed on <https://docs.x.ai/docs/models> and
/// <https://docs.x.ai/developers/grok-4-7> (model id `grok-4.7`, Responses API,
/// 500k context). Upstream `default_models.json` in this tree still stops at
/// grok-4.6, so 4.7 is added in front of that catalog.
pub const OFFLINE_DEFAULT_MODEL: &str = "grok-4.7";

/// `xai_grok_login::CLI_CHAT_PROXY_BASE_URL_DEFAULT`, which session tokens are sent to.
pub const CLI_CHAT_PROXY_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";

const EXCLUDED_TOKENS: &[&str] = &[
    "mini",
    "vision",
    "image",
    "imagine",
    "video",
    "voice",
    "audio",
    "tts",
    "stt",
    "embed",
    "embedding",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ModelChoice {
    pub latest: String,
    pub models: Vec<String>,
    pub entries: Vec<ModelInfo>,
}

/// Credentials for listing models.
#[derive(Debug, Clone, Copy)]
pub enum ModelListAuth<'a> {
    ApiKey {
        base_url: &'a str,
        key: &'a str,
    },
    /// A Grok sign-in token, sent the way the CLI sends it.
    Session {
        token: &'a str,
        user_id: Option<&'a str>,
        email: Option<&'a str>,
        client_version: &'a str,
    },
}

impl ModelListAuth<'_> {
    fn is_session(&self) -> bool {
        matches!(self, Self::Session { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelCache {
    fetched_at_unix: u64,
    /// Ids only; older builds read just this field.
    models: Vec<String>,
    #[serde(default)]
    entries: Vec<ModelInfo>,
}

/// Versions compare as decimal numbers, so 4.7 outranks 4.20 and 4.6.
/// That matches xAI's lineup: grok-4.7 is newer than the grok-4.20 line.
/// Hyphen tokens `mini`, `vision`, `image`, `imagine`, `video`, `voice`,
/// `audio`, `tts`, `stt`, and `embed` are skipped unless every candidate is one
/// of those. An unsuffixed id (`grok-4.7`) is preferred over every variant
/// (`grok-4.7-fast`, `grok-4.8-fast`) when one is listed.
pub fn choose_latest(ids: &[String]) -> Option<String> {
    let parsed: Vec<Candidate> = ids.iter().filter_map(|id| classify(id)).collect();
    let general: Vec<&Candidate> = parsed
        .iter()
        .filter(|candidate| !candidate.excluded)
        .collect();
    let pool: Vec<&Candidate> = if general.iter().any(|candidate| candidate.bare) {
        general
            .into_iter()
            .filter(|candidate| candidate.bare)
            .collect()
    } else if !general.is_empty() {
        general
    } else {
        parsed.iter().collect()
    };
    pool.into_iter()
        .max_by(|left, right| {
            left.version
                .total_cmp(&right.version)
                .then(left.bare.cmp(&right.bare))
                .then(right.id.len().cmp(&left.id.len()))
                .then(right.id.cmp(&left.id))
        })
        .map(|candidate| candidate.id.clone())
}

/// The built-in Grok catalog, newest first.
pub fn offline_entries() -> Vec<ModelInfo> {
    known_models()
        .into_iter()
        .filter(|entry| entry.id.starts_with("grok-"))
        .collect()
}

pub fn offline_choice() -> ModelChoice {
    ModelChoice::from_entries(offline_entries(), true)
}

impl ModelChoice {
    /// A list of ids with no metadata (an old cache, a test). Known ids get
    /// the built-in metadata and the offline catalog is kept alongside.
    pub fn from_primary(primary: &[String]) -> Self {
        let mut entries: Vec<ModelInfo> = primary.iter().map(|id| ModelInfo::bare(id)).collect();
        for entry in offline_entries() {
            if !entries.iter().any(|existing| existing.id == entry.id) {
                entries.push(entry);
            }
        }
        let mut choice = Self::from_entries(entries, true);
        choice
            .entries
            .sort_by_key(|entry| std::cmp::Reverse(model_rank(&entry.id)));
        choice.models = choice
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        choice
    }

    /// A `/v1/models` list as the CLI offers it: the server's entries in the
    /// server's order, each completed from the built-in catalog.
    pub fn from_entries(entries: Vec<ModelInfo>, session_auth: bool) -> Self {
        let known = known_models();
        let mut kept: Vec<ModelInfo> = Vec::new();
        for mut entry in entries {
            if !entry.visible_for_auth(session_auth)
                || kept.iter().any(|existing| existing.id == entry.id)
            {
                continue;
            }
            if let Some(builtin) = known.iter().find(|builtin| builtin.id == entry.id) {
                entry.fill_from(builtin);
            }
            kept.push(entry);
        }
        if kept.is_empty() {
            kept = offline_entries();
        }
        let ids: Vec<String> = kept.iter().map(|entry| entry.id.clone()).collect();
        let latest = choose_latest(&ids)
            .or_else(|| ids.first().cloned())
            .unwrap_or_else(|| OFFLINE_DEFAULT_MODEL.to_string());
        Self {
            latest,
            models: ids,
            entries: kept,
        }
    }
}

/// Point the xAI preset at the cached or offline newest model when it is set
/// to follow latest. A pinned model is left alone.
pub fn apply_known_models(store: &mut ProviderStore) {
    apply_choice(store, &choice_from_cache_or_offline());
}

pub fn apply_choice(store: &mut ProviderStore, choice: &ModelChoice) {
    let Some(provider) = store
        .providers
        .iter_mut()
        .find(|provider| provider.id == "xai")
    else {
        return;
    };
    let mut models = choice.models.clone();
    let mut entries = choice.entries.clone();
    if provider.follow_latest {
        provider.model = choice.latest.clone();
    }
    if !provider.model.is_empty() && !models.iter().any(|id| id == &provider.model) {
        models.push(provider.model.clone());
        entries.push(
            crate::models::known_model(&provider.model)
                .unwrap_or_else(|| ModelInfo::bare(&provider.model)),
        );
    }
    provider.models = models.clone();
    provider.model_hints = models;
    provider.model_info = entries;
}

pub fn choice_from_cache_or_offline() -> ModelChoice {
    match read_cache() {
        Some(cache) if !cache.models.is_empty() || !cache.entries.is_empty() => {
            if !cache_is_fresh(cache.fetched_at_unix) {
                tracing::debug!(
                    ttl_secs = MODEL_CACHE_TTL.as_secs(),
                    "xAI model cache is older than the refresh interval; using it until the next refresh"
                );
            }
            if cache.entries.is_empty() {
                ModelChoice::from_primary(&cache.models)
            } else {
                // The cache holds what this login was allowed to see.
                ModelChoice::from_entries(cache.entries, true)
            }
        }
        _ => offline_choice(),
    }
}

pub fn cache_path() -> std::path::PathBuf {
    grok_home().join("xai-model-cache.json")
}

/// List models with an API key against `base_url`. Kept for callers that only
/// have a key; [`refresh_xai_model_list`] also takes a Grok session.
pub async fn refresh_xai_models(base_url: &str, api_key: Option<&str>) -> ModelChoice {
    match api_key.map(str::trim).filter(|value| !value.is_empty()) {
        Some(key) => refresh_xai_model_list(ModelListAuth::ApiKey { base_url, key }).await,
        None => choice_from_cache_or_offline(),
    }
}

/// Fetch `/v1/models`, cache it, and return the choice. A failure falls back
/// to the cache and then the built-in catalog.
pub async fn refresh_xai_model_list(auth: ModelListAuth<'_>) -> ModelChoice {
    match fetch_model_entries(auth).await {
        Ok(entries) if !entries.is_empty() => {
            let choice = ModelChoice::from_entries(entries, auth.is_session());
            if let Err(error) = write_cache(&choice.entries) {
                tracing::debug!(%error, "xAI model cache was not written");
            }
            choice
        }
        Ok(_) => choice_from_cache_or_offline(),
        Err(error) => {
            tracing::warn!(%error, "xAI /v1/models refresh failed");
            choice_from_cache_or_offline()
        }
    }
}

pub fn refresh_xai_models_blocking(base_url: &str, api_key: Option<&str>) -> ModelChoice {
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) {
        match fetch_model_entries_blocking(base_url, api_key) {
            Ok(entries) if !entries.is_empty() => {
                let choice = ModelChoice::from_entries(entries, false);
                let _ = write_cache(&choice.entries);
                return choice;
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "xAI /v1/models refresh failed"),
        }
    }
    choice_from_cache_or_offline()
}

fn fetch_model_entries_blocking(base_url: &str, api_key: &str) -> Result<Vec<ModelInfo>, String> {
    let url =
        crate::translate::endpoint_url(base_url, "models").map_err(|error| error.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body: serde_json::Value = response.json().map_err(|error| error.to_string())?;
    Ok(parse_model_entries(&body))
}

/// `GET {base}/models` for any provider. OpenAI, Responses, and xAI use a bearer
/// token. Anthropic Messages uses `x-api-key`.
pub async fn fetch_provider_models(
    base_url: &str,
    auth: crate::types::AuthStyle,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let url =
        crate::translate::endpoint_url(base_url, "models").map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(url);
    request = match auth {
        crate::types::AuthStyle::Bearer => request.bearer_auth(api_key),
        crate::types::AuthStyle::XApiKey => request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
    };
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    let ids = parse_models_payload(&body);
    if ids.is_empty() {
        return Err("no models returned".into());
    }
    Ok(ids)
}

/// The request the CLI's `OaiModelSource::fetch` makes for this auth.
pub fn model_list_request(
    client: &reqwest::Client,
    auth: ModelListAuth<'_>,
) -> Result<reqwest::RequestBuilder, String> {
    Ok(match auth {
        ModelListAuth::ApiKey { base_url, key } => {
            let url = crate::translate::endpoint_url(base_url, "models")
                .map_err(|error| error.to_string())?;
            client.get(url).bearer_auth(key)
        }
        ModelListAuth::Session {
            token,
            user_id,
            email,
            client_version,
        } => {
            let url = crate::translate::endpoint_url(CLI_CHAT_PROXY_BASE_URL, "models")
                .map_err(|error| error.to_string())?;
            let mut request = client
                .get(url)
                .bearer_auth(token)
                .header("X-XAI-Token-Auth", "xai-grok-cli")
                .header("x-grok-client-version", client_version)
                .header("x-grok-client-mode", "interactive");
            if let Some(user_id) = user_id.filter(|value| !value.is_empty()) {
                request = request.header("x-userid", user_id);
            }
            if let Some(email) = email.filter(|value| !value.is_empty()) {
                request = request.header("x-email", email);
            }
            request
        }
    })
}

pub async fn fetch_model_entries(auth: ModelListAuth<'_>) -> Result<Vec<ModelInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|error| error.to_string())?;
    let response = model_list_request(&client, auth)?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    Ok(parse_model_entries(&body))
}

pub fn parse_models_payload(body: &serde_json::Value) -> Vec<String> {
    parse_model_entries(body)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

fn read_cache() -> Option<ModelCache> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(entries: &[ModelInfo]) -> Result<(), String> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let cache = ModelCache {
        fetched_at_unix: unix_now(),
        models: entries.iter().map(|entry| entry.id.clone()).collect(),
        entries: entries.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

pub fn cache_is_fresh(fetched_at_unix: u64) -> bool {
    unix_now().saturating_sub(fetched_at_unix) < MODEL_CACHE_TTL.as_secs()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

struct Candidate {
    id: String,
    version: f64,
    excluded: bool,
    bare: bool,
}

fn classify(id: &str) -> Option<Candidate> {
    let rest = id.trim().strip_prefix("grok-")?;
    let (numeric, suffix) = split_version(rest)?;
    let version = numeric.parse::<f64>().ok()?;
    if !version.is_finite() {
        return None;
    }
    let tokens: Vec<&str> = suffix
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let excluded = tokens
        .iter()
        .any(|token| EXCLUDED_TOKENS.contains(&token.to_ascii_lowercase().as_str()));
    Some(Candidate {
        id: id.trim().to_string(),
        version,
        excluded,
        bare: tokens.is_empty(),
    })
}

fn split_version(rest: &str) -> Option<(&str, &str)> {
    let mut end = 0;
    let mut seen_dot = false;
    for (index, character) in rest.char_indices() {
        if character.is_ascii_digit() {
            end = index + character.len_utf8();
            continue;
        }
        if character == '.' && !seen_dot {
            seen_dot = true;
            end = index + 1;
            continue;
        }
        break;
    }
    if end == 0 || rest.as_bytes().get(end.saturating_sub(1)) == Some(&b'.') {
        return None;
    }
    Some((&rest[..end], &rest[end..]))
}

fn model_rank(id: &str) -> (u8, i64, u8, isize) {
    match classify(id) {
        Some(candidate) => (
            if candidate.excluded { 0 } else { 1 },
            (candidate.version * 1000.0).round() as i64,
            if candidate.bare { 1 } else { 0 },
            -(candidate.id.len() as isize),
        ),
        None => (0, 0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReasoningEffort;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn newest_general_model_outranks_older_and_specialized_ids() {
        let listed = ids(&[
            "grok-4.5",
            "grok-4.20-0309-reasoning",
            "grok-4.6",
            "grok-4.7-fast",
            "grok-4.7",
            "grok-2-vision-1212",
            "grok-imagine-image",
            "grok-3-mini",
        ]);
        assert_eq!(choose_latest(&listed).as_deref(), Some("grok-4.7"));
    }

    #[test]
    fn a_newer_variant_does_not_replace_the_newest_full_model() {
        let listed = ids(&["grok-4.7", "grok-4.8-fast"]);
        assert_eq!(choose_latest(&listed).as_deref(), Some("grok-4.7"));
        let only_variants = ids(&["grok-4.7-fast", "grok-4.8-fast"]);
        assert_eq!(
            choose_latest(&only_variants).as_deref(),
            Some("grok-4.8-fast")
        );
    }

    #[test]
    fn specialized_models_are_used_only_when_nothing_else_is_listed() {
        let listed = ids(&["grok-2-vision-1212", "grok-imagine-image"]);
        assert_eq!(
            choose_latest(&listed).as_deref(),
            Some("grok-2-vision-1212")
        );
    }

    #[test]
    fn offline_fallback_is_grok_4_7_and_offers_the_fast_variant() {
        let choice = offline_choice();
        assert_eq!(choice.latest, "grok-4.7");
        assert_eq!(
            choice.models,
            ids(&["grok-4.7", "grok-4.7-fast", "grok-4.6", "grok-4.5"])
        );
        let fast = choice
            .entries
            .iter()
            .find(|entry| entry.id == "grok-4.7-fast")
            .unwrap();
        assert_eq!(fast.display_name(), "Grok 4.7 Fast");
    }

    #[test]
    fn models_payload_reads_id_or_model() {
        let body = serde_json::json!({
            "data": [
                {"id": "grok-4.7"},
                {"model": "grok-4.6", "context_window": 500000}
            ]
        });
        assert_eq!(
            parse_models_payload(&body),
            vec!["grok-4.7".to_string(), "grok-4.6".to_string()]
        );
    }

    #[test]
    fn a_server_list_is_offered_as_listed_with_fast_variants() {
        let body = serde_json::json!({
            "data": [
                {"id": "grok-4.7", "name": "Grok 4.7"},
                {"id": "grok-4.7-fast", "name": "Grok 4.7 Fast"},
                {"id": "grok-4.6"},
                {"id": "grok-internal", "supportedInApi": false},
                {"id": "grok-hidden", "hidden": true}
            ]
        });
        let api_key = ModelChoice::from_entries(parse_model_entries(&body), false);
        assert_eq!(
            api_key.models,
            ids(&["grok-4.7", "grok-4.7-fast", "grok-4.6"])
        );
        assert_eq!(api_key.latest, "grok-4.7");
        let grok_46 = api_key
            .entries
            .iter()
            .find(|entry| entry.id == "grok-4.6")
            .unwrap();
        assert_eq!(grok_46.display_name(), "Grok 4.6");
        assert_eq!(grok_46.reasoning_effort, Some(ReasoningEffort::High));

        let session = ModelChoice::from_entries(parse_model_entries(&body), true);
        assert!(session.models.iter().any(|id| id == "grok-internal"));
        assert!(!session.models.iter().any(|id| id == "grok-hidden"));
    }

    #[test]
    fn follow_latest_tracks_the_choice_and_a_pin_does_not() {
        let mut store = crate::presets::default_store();
        let choice = ModelChoice::from_primary(&ids(&["grok-5", "grok-4.7"]));
        apply_choice(&mut store, &choice);
        let xai = store
            .providers
            .iter()
            .find(|provider| provider.id == "xai")
            .unwrap();
        assert_eq!(xai.model, "grok-5");
        assert!(xai.model_hints.iter().any(|id| id == "grok-4.5"));
        assert!(xai.model_hints.iter().any(|id| id == "grok-4.7-fast"));
        assert!(xai.model_entry("grok-4.7-fast").is_some());

        let mut pinned = store.clone();
        let xai = pinned
            .providers
            .iter_mut()
            .find(|provider| provider.id == "xai")
            .unwrap();
        xai.follow_latest = false;
        xai.model = "grok-4.5".into();
        apply_choice(&mut pinned, &choice);
        let xai = pinned
            .providers
            .iter()
            .find(|provider| provider.id == "xai")
            .unwrap();
        assert_eq!(xai.model, "grok-4.5");
    }

    #[test]
    fn a_session_lists_models_through_the_cli_proxy_with_cli_headers() {
        let client = reqwest::Client::new();
        let request = model_list_request(
            &client,
            ModelListAuth::Session {
                token: "tok",
                user_id: Some("user-1"),
                email: Some("a@example.com"),
                client_version: "0.3.0",
            },
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://cli-chat-proxy.grok.com/v1/models"
        );
        let headers = request.headers();
        assert_eq!(headers["authorization"], "Bearer tok");
        assert_eq!(headers["x-xai-token-auth"], "xai-grok-cli");
        assert_eq!(headers["x-userid"], "user-1");
        assert_eq!(headers["x-email"], "a@example.com");
        assert_eq!(headers["x-grok-client-mode"], "interactive");

        let request = model_list_request(
            &client,
            ModelListAuth::ApiKey {
                base_url: "https://api.x.ai/v1",
                key: "xai-key",
            },
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.url().as_str(), "https://api.x.ai/v1/models");
        assert!(request.headers().get("x-xai-token-auth").is_none());
    }
}
