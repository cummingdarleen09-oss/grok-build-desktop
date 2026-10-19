//! Which Grok model the xAI preset calls.
//!
//! The preset does not pin a version in source. When `GET /v1/models` succeeds,
//! the newest general chat model in that list wins. Otherwise the choice is the
//! cached list, then the offline fallback (grok-4.7, confirmed on docs.x.ai,
//! then the ids in upstream `default_models.json`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config_merge::grok_home;
use crate::types::ProviderStore;

/// How long a `/v1/models` response stays fresh for the CLI.
/// The desktop app refreshes on every launch and writes this cache.
pub const MODEL_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Confirmed on <https://docs.x.ai/docs/models> and
/// <https://docs.x.ai/developers/grok-4-7> (model id `grok-4.7`, Responses API,
/// 500k context). Upstream `default_models.json` in this tree still stops at
/// grok-4.6, so 4.7 is added in front of that catalog.
pub const OFFLINE_DEFAULT_MODEL: &str = "grok-4.7";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub latest: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelCache {
    fetched_at_unix: u64,
    models: Vec<String>,
}

/// Versions compare as decimal numbers, so 4.7 outranks 4.20 and 4.6.
/// That matches xAI's lineup: grok-4.7 is newer than the grok-4.20 line.
/// Hyphen tokens `mini`, `vision`, `image`, `imagine`, `video`, `voice`,
/// `audio`, `tts`, `stt`, and `embed` are skipped unless every candidate is one
/// of those. An unsuffixed id wins ties (`grok-4.7` before `grok-4.7-fast`).
pub fn choose_latest(ids: &[String]) -> Option<String> {
    let parsed: Vec<Candidate> = ids.iter().filter_map(|id| classify(id)).collect();
    let pool: Vec<&Candidate> = if parsed.iter().any(|candidate| !candidate.excluded) {
        parsed
            .iter()
            .filter(|candidate| !candidate.excluded)
            .collect()
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

pub fn offline_candidates() -> Vec<String> {
    let mut ids = vec![OFFLINE_DEFAULT_MODEL.to_string()];
    if let Ok(value) =
        serde_json::from_str::<serde_json::Value>(xai_grok_models::DEFAULT_MODELS_JSON)
    {
        if let Some(models) = value.get("models").and_then(|entry| entry.as_array()) {
            for model in models {
                if let Some(id) = model.get("model").and_then(|entry| entry.as_str()) {
                    if !ids.iter().any(|existing| existing == id) {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    ids
}

pub fn offline_choice() -> ModelChoice {
    ModelChoice::from_primary(&offline_candidates())
}

impl ModelChoice {
    pub fn from_primary(primary: &[String]) -> Self {
        let latest = choose_latest(primary)
            .or_else(|| choose_latest(&offline_candidates()))
            .unwrap_or_else(|| OFFLINE_DEFAULT_MODEL.to_string());
        let mut models = Vec::new();
        for id in primary.iter().chain(offline_candidates().iter()) {
            if !models.iter().any(|existing| existing == id) {
                models.push(id.clone());
            }
        }
        if !models.iter().any(|existing| existing == &latest) {
            models.insert(0, latest.clone());
        }
        models.sort_by(|left, right| model_rank(right).cmp(&model_rank(left)));
        Self { latest, models }
    }
}

/// Point the xAI preset at the cached or offline newest model when it is set
/// to follow latest. A pinned model is left alone. Hints always keep the
/// offline ids so 4.5 and 4.6 stay selectable.
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
    if !provider.model.is_empty() && !models.iter().any(|id| id == &provider.model) {
        models.push(provider.model.clone());
    }
    provider.models = models.clone();
    provider.model_hints = models;
    if provider.follow_latest {
        provider.model = choice.latest.clone();
    }
}

pub fn choice_from_cache_or_offline() -> ModelChoice {
    match read_cache() {
        Some(cache) if !cache.models.is_empty() => {
            if !cache_is_fresh(cache.fetched_at_unix) {
                tracing::debug!(
                    ttl_secs = MODEL_CACHE_TTL.as_secs(),
                    "xAI model cache is older than the refresh interval; using it until the next refresh"
                );
            }
            ModelChoice::from_primary(&cache.models)
        }
        _ => offline_choice(),
    }
}

pub fn cache_path() -> std::path::PathBuf {
    grok_home().join("xai-model-cache.json")
}

pub async fn refresh_xai_models(base_url: &str, api_key: Option<&str>) -> ModelChoice {
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) {
        match fetch_model_ids(base_url, api_key).await {
            Ok(models) if !models.is_empty() => {
                let _ = write_cache(&models);
                return ModelChoice::from_primary(&models);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "xAI /v1/models refresh failed"),
        }
    }
    choice_from_cache_or_offline()
}

pub fn refresh_xai_models_blocking(base_url: &str, api_key: Option<&str>) -> ModelChoice {
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) {
        match fetch_model_ids_blocking(base_url, api_key) {
            Ok(models) if !models.is_empty() => {
                let _ = write_cache(&models);
                return ModelChoice::from_primary(&models);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "xAI /v1/models refresh failed"),
        }
    }
    choice_from_cache_or_offline()
}

fn fetch_model_ids_blocking(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
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
    Ok(parse_models_payload(&body))
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

pub async fn fetch_model_ids(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let url =
        crate::translate::endpoint_url(base_url, "models").map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    Ok(parse_models_payload(&body))
}

pub fn parse_models_payload(body: &serde_json::Value) -> Vec<String> {
    let Some(data) = body.get("data").and_then(|entry| entry.as_array()) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(|entry| entry.as_str())
            .or_else(|| item.get("model").and_then(|entry| entry.as_str()));
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) {
            if !ids.iter().any(|existing| existing == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

fn read_cache() -> Option<ModelCache> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(models: &[String]) -> Result<(), String> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let cache = ModelCache {
        fetched_at_unix: unix_now(),
        models: models.to_vec(),
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
    fn specialized_models_are_used_only_when_nothing_else_is_listed() {
        let listed = ids(&["grok-2-vision-1212", "grok-imagine-image"]);
        assert_eq!(
            choose_latest(&listed).as_deref(),
            Some("grok-2-vision-1212")
        );
    }

    #[test]
    fn offline_fallback_is_grok_4_7_and_keeps_upstream_catalog() {
        let choice = offline_choice();
        assert_eq!(choice.latest, "grok-4.7");
        assert!(choice.models.iter().any(|id| id == "grok-4.6"));
        assert!(choice.models.iter().any(|id| id == "grok-4.5"));
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
}
