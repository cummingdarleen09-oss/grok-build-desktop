//! Per-model metadata: display name, context windows, and reasoning effort.
//!
//! Entries are read the way the grok-build CLI reads `/v1/models`
//! (`xai-grok-shell` `remote::client::parse_remote_model_value`): every entry
//! in `data[]` is a separate model, so a fast variant such as "Grok 4.7 Fast"
//! is its own id with its own name, never a flag on grok-4.7. The effort and
//! context-window rules follow `agent::config` in the same crate.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use xai_grok_sampling_types::{
    ReasoningEffort, ReasoningEffortOption, effort_label, parse_reasoning_effort_options,
};

use crate::types::{ProviderConfig, ProviderFormat};

/// `xai-grok-shell` `remote::client::DEFAULT_CONTEXT_WINDOW`.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 256_000;
/// `xai-grok-compaction` `DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT`.
pub const DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT: u8 = 85;

/// The CLI's effort menu for a model that supports effort but lists no menu
/// (`ModelsManager::model_supports_reasoning_effort_value`).
const FALLBACK_EFFORTS: [ReasoningEffort; 4] = [
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
];

/// How an Anthropic Messages request asks for thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingStyle {
    /// `thinking: {type: "adaptive"}` plus `output_config.effort`, as the CLI sends.
    #[default]
    Adaptive,
    /// `thinking: {type: "enabled", budget_tokens}` for models without adaptive thinking.
    Budget,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The id sent as `model`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Default context window in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Every window the user can pick, the default included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_windows: Vec<u64>,
    /// `None` means not stated, which the CLI reads as "yes" for Anthropic
    /// Messages and "no" for everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    /// The model's default effort when no menu entry is marked default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_efforts: Vec<ReasoningEffortOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_threshold_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// `Some(false)` hides the model from API-key users; Grok sign-in still sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_in_api: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingStyle>,
}

/// What the composer offers for one model, and what a request falls back to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelOptions {
    pub efforts: Vec<ReasoningEffortOption>,
    pub default_effort: Option<ReasoningEffort>,
    pub context_windows: Vec<u64>,
    pub default_context_window: u64,
    pub auto_compact_threshold_percent: u8,
    pub thinking: ThinkingStyle,
}

impl ModelInfo {
    pub fn bare(id: &str) -> Self {
        Self {
            id: id.to_string(),
            ..Self::default()
        }
    }

    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.id)
    }

    /// The CLI's `ModelInfo::visible_for_auth`.
    pub fn visible_for_auth(&self, session_auth: bool) -> bool {
        !self.hidden && (session_auth || self.supported_in_api != Some(false))
    }

    /// Fill what this entry does not say from `known`. A server entry wins
    /// field by field; the built-in catalog covers what it leaves out.
    pub fn fill_from(&mut self, known: &ModelInfo) {
        if self.name.is_none() {
            self.name = known.name.clone();
        }
        if self.context_window.is_none() && self.context_windows.is_empty() {
            self.context_window = known.context_window;
            self.context_windows = known.context_windows.clone();
        }
        let says_nothing_about_effort = self.supports_reasoning_effort.is_none()
            && self.reasoning_effort.is_none()
            && self.reasoning_efforts.is_empty();
        if says_nothing_about_effort {
            self.supports_reasoning_effort = known.supports_reasoning_effort;
            self.reasoning_effort = known.reasoning_effort;
            self.reasoning_efforts = known.reasoning_efforts.clone();
        }
        if self.auto_compact_threshold_percent.is_none() {
            self.auto_compact_threshold_percent = known.auto_compact_threshold_percent;
        }
        if self.thinking.is_none() {
            self.thinking = known.thinking;
        }
    }

    pub fn options(&self, format: ProviderFormat, provider_window: u64) -> ModelOptions {
        let listed = !self.reasoning_efforts.is_empty();
        let supports = listed
            || self
                .supports_reasoning_effort
                .unwrap_or(format == ProviderFormat::Anthropic);
        let efforts: Vec<ReasoningEffortOption> = if !supports {
            Vec::new()
        } else if listed {
            self.reasoning_efforts.clone()
        } else {
            FALLBACK_EFFORTS
                .iter()
                .map(|value| ReasoningEffortOption {
                    id: value.as_ref().to_string(),
                    value: *value,
                    label: effort_label(*value),
                    description: None,
                    default: self.reasoning_effort == Some(*value),
                })
                .collect()
        };
        let default_effort = efforts
            .iter()
            .find(|option| option.default)
            .map(|option| option.value)
            .or(self
                .reasoning_effort
                .filter(|value| efforts.iter().any(|option| option.value == *value)));
        let fallback_window = if provider_window > 0 {
            provider_window
        } else {
            DEFAULT_CONTEXT_WINDOW
        };
        let (default_context_window, context_windows) =
            context_window_choices(self.context_window, &self.context_windows)
                .unwrap_or((fallback_window, Vec::new()));
        let context_windows = if context_windows.is_empty() {
            vec![default_context_window]
        } else {
            context_windows
        };
        ModelOptions {
            efforts,
            default_effort,
            context_windows,
            default_context_window,
            auto_compact_threshold_percent: self
                .auto_compact_threshold_percent
                .filter(|percent| (1..=100).contains(percent))
                .unwrap_or(DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT),
            thinking: self.thinking.unwrap_or_default(),
        }
    }
}

impl ModelOptions {
    /// `wanted` when this model offers it, otherwise the model default.
    pub fn effort(&self, wanted: Option<ReasoningEffort>) -> Option<ReasoningEffort> {
        wanted
            .filter(|value| self.efforts.iter().any(|option| option.value == *value))
            .or(self.default_effort)
    }

    /// `wanted` when this model offers it, otherwise the model default.
    pub fn context_window(&self, wanted: Option<u64>) -> u64 {
        wanted
            .filter(|window| self.context_windows.contains(window))
            .unwrap_or(self.default_context_window)
    }
}

impl ProviderConfig {
    pub fn model_entry(&self, model: &str) -> Option<&ModelInfo> {
        self.model_info.iter().find(|entry| entry.id == model)
    }

    /// Composer options for `model`. A model without an entry gets no effort
    /// menu and the provider's context window.
    pub fn model_options(&self, model: &str) -> ModelOptions {
        match self.model_entry(model) {
            Some(entry) => entry.options(self.format, self.context_window),
            None => ModelInfo::bare(model).options(self.format, self.context_window),
        }
    }
}

/// `xai-grok-shell` `agent::config::context_window_choices`: the default is
/// the scalar window, else the first listed one; the menu is the default
/// (when it is not listed) followed by the listed windows.
pub fn context_window_choices(scalar: Option<u64>, listed: &[u64]) -> Option<(u64, Vec<u64>)> {
    let listed: Vec<u64> = listed
        .iter()
        .copied()
        .filter(|window| *window > 0)
        .collect();
    let default = scalar
        .filter(|window| *window > 0)
        .or_else(|| listed.first().copied())?;
    if listed.is_empty() {
        return Some((default, Vec::new()));
    }
    let mut choices = Vec::with_capacity(listed.len() + 1);
    if !listed.contains(&default) {
        choices.push(default);
    }
    for window in listed {
        if !choices.contains(&window) {
            choices.push(window);
        }
    }
    Some((default, choices))
}

/// One `data[]` entry of a `/v1/models` response, or a `default_models.json`
/// entry. Keys are read in the CLI's order, camelCase first.
pub fn parse_model_value(value: &Value) -> Option<ModelInfo> {
    let obj = value.as_object()?;
    let meta = obj.get("_meta").and_then(Value::as_object);
    let id = string(obj, "model")
        .or_else(|| string(obj, "modelId"))
        .or_else(|| string(obj, "id"))
        .or_else(|| meta.and_then(|m| string(m, "model")))
        .or_else(|| meta.and_then(|m| string(m, "modelId")))?;
    let listed = u64_array(obj, "contextWindows")
        .or_else(|| u64_array(obj, "context_windows"))
        .or_else(|| meta.and_then(|m| u64_array(m, "contextWindows")))
        .unwrap_or_default();
    let scalar = number(obj, "contextWindow")
        .or_else(|| number(obj, "context_window"))
        .or_else(|| meta.and_then(|m| number(m, "contextWindow")))
        .or_else(|| meta.and_then(|m| number(m, "totalContextTokens")));
    let (context_window, context_windows) = match context_window_choices(scalar, &listed) {
        Some((default, choices)) => (Some(default), choices),
        None => (None, Vec::new()),
    };
    let (reasoning_efforts, server_default) = obj
        .get("reasoningEfforts")
        .or_else(|| obj.get("reasoning_efforts"))
        .or_else(|| meta.and_then(|m| m.get("reasoningEfforts")))
        .and_then(Value::as_array)
        .map(|items| parse_reasoning_effort_options(items, "reasoningEfforts"))
        .filter(|options| !options.is_empty())
        .map(|options| (options, false))
        .or_else(|| {
            obj.get("capabilities")
                .and_then(Value::as_object)
                .and_then(capabilities_efforts)
        })
        .unwrap_or_default();
    let mut reasoning_effort = string(obj, "reasoningEffort")
        .or_else(|| string(obj, "reasoning_effort"))
        .or_else(|| meta.and_then(|m| string(m, "reasoningEffort")))
        .and_then(|text| text.parse().ok());
    let mut supports_reasoning_effort = obj
        .get("supportsReasoningEffort")
        .or_else(|| obj.get("supports_reasoning_effort"))
        .or_else(|| meta.and_then(|m| m.get("supportsReasoningEffort")))
        .and_then(Value::as_bool);
    // `ModelInfo::derive_reasoning_effort_fields`: a menu means effort is supported,
    // and its default is the marked entry, else the first one.
    if !reasoning_efforts.is_empty() {
        supports_reasoning_effort = Some(true);
        if reasoning_effort.is_none() {
            let first = if server_default {
                None
            } else {
                reasoning_efforts.first()
            };
            reasoning_effort = reasoning_efforts
                .iter()
                .find(|option| option.default)
                .or(first)
                .map(|option| option.value);
        }
    }
    let flag = |key_camel: &str, key_snake: &str| {
        obj.get(key_camel)
            .or_else(|| obj.get(key_snake))
            .or_else(|| meta.and_then(|m| m.get(key_camel)))
            .and_then(Value::as_bool)
    };
    Some(ModelInfo {
        name: string(obj, "name"),
        context_window,
        context_windows,
        supports_reasoning_effort,
        reasoning_effort,
        reasoning_efforts,
        auto_compact_threshold_percent: number(obj, "autoCompactThresholdPercent")
            .or_else(|| number(obj, "auto_compact_threshold_percent"))
            .and_then(|value| u8::try_from(value).ok()),
        hidden: flag("hidden", "hidden").unwrap_or(false),
        supported_in_api: flag("supportedInApi", "supported_in_api"),
        thinking: None,
        id,
    })
}

/// Every parseable entry of a `/v1/models` body, in the server's order.
pub fn parse_model_entries(body: &Value) -> Vec<ModelInfo> {
    let Some(data) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut entries: Vec<ModelInfo> = Vec::new();
    for item in data {
        let Some(entry) = parse_model_value(item) else {
            continue;
        };
        if !entries.iter().any(|existing| existing.id == entry.id) {
            entries.push(entry);
        }
    }
    entries
}

/// The public `/v1/models` lists efforts under `capabilities`
/// (`remote::client::parse_capabilities_reasoning_efforts`). The flag is true
/// when no listed level is the default, so the server picks.
fn capabilities_efforts(caps: &Map<String, Value>) -> Option<(Vec<ReasoningEffortOption>, bool)> {
    let items = caps.get("reasoning_effort")?.as_array()?;
    let mut options = parse_reasoning_effort_options(items, "capabilities.reasoning_effort");
    let mut marked = false;
    if let Some(default) = caps
        .get("default_reasoning_effort")
        .filter(|value| !value.is_null())
    {
        let level = default
            .as_str()
            .and_then(|text| text.parse::<ReasoningEffort>().ok());
        for option in &mut options {
            option.default = level == Some(option.value);
            marked |= option.default;
        }
    }
    Some((options, !marked))
}

/// Built-in metadata. grok-4.6 and grok-4.5 come from the CLI's
/// `default_models.json`. grok-4.7 is confirmed on docs.x.ai (500k context)
/// and offered with grok-4.6's effort menu. "Grok 4.7 Fast" is listed by the
/// Grok backend as its own model; offline it carries no effort menu, so
/// nothing is sent until `/v1/models` says otherwise. The Claude rows use
/// budget thinking because those models have no adaptive thinking.
pub fn known_models() -> Vec<ModelInfo> {
    let mut known = Vec::new();
    let upstream: Vec<ModelInfo> =
        serde_json::from_str::<Value>(xai_grok_models::DEFAULT_MODELS_JSON)
            .ok()
            .and_then(|value| {
                value.get("models").and_then(Value::as_array).map(|items| {
                    items
                        .iter()
                        .filter_map(parse_model_value)
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
    let newest_menu = upstream
        .iter()
        .find(|entry| entry.id == "grok-4.6")
        .map(|entry| (entry.reasoning_effort, entry.reasoning_efforts.clone()));
    let mut grok_47 = ModelInfo {
        name: Some("Grok 4.7".into()),
        context_window: Some(500_000),
        supports_reasoning_effort: Some(true),
        auto_compact_threshold_percent: Some(80),
        ..ModelInfo::bare("grok-4.7")
    };
    if let Some((default, menu)) = newest_menu {
        grok_47.reasoning_effort = default;
        grok_47.reasoning_efforts = menu;
    }
    known.push(grok_47);
    known.push(ModelInfo {
        name: Some("Grok 4.7 Fast".into()),
        context_window: Some(500_000),
        ..ModelInfo::bare("grok-4.7-fast")
    });
    known.extend(upstream);
    for (id, name) in [
        ("claude-sonnet-4-5", "Claude Sonnet 4.5"),
        ("claude-opus-4-5", "Claude Opus 4.5"),
        ("claude-haiku-4-5", "Claude Haiku 4.5"),
    ] {
        known.push(ModelInfo {
            name: Some(name.into()),
            context_window: Some(200_000),
            thinking: Some(ThinkingStyle::Budget),
            ..ModelInfo::bare(id)
        });
    }
    known.push(ModelInfo {
        context_window: Some(200_000),
        supports_reasoning_effort: Some(true),
        reasoning_efforts: [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]
        .into_iter()
        .map(|value| ReasoningEffortOption {
            id: value.as_ref().to_string(),
            value,
            label: effort_label(value),
            description: None,
            default: value == ReasoningEffort::Medium,
        })
        .collect(),
        reasoning_effort: Some(ReasoningEffort::Medium),
        ..ModelInfo::bare("o3")
    });
    known
}

pub fn known_model(id: &str) -> Option<ModelInfo> {
    known_models().into_iter().find(|entry| entry.id == id)
}

/// Give every listed model without an entry the built-in metadata, when
/// there is one, so settings start from the CLI's defaults.
pub fn seed_known_entries(provider: &mut ProviderConfig) {
    let known = known_models();
    for id in provider.models.clone() {
        if provider.model_entry(&id).is_some() {
            continue;
        }
        if let Some(entry) = known.iter().find(|entry| entry.id == id) {
            provider.model_info.push(entry.clone());
        }
    }
}

fn string(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn number(obj: &Map<String, Value>, key: &str) -> Option<u64> {
    obj.get(key).and_then(Value::as_u64)
}

fn u64_array(obj: &Map<String, Value>, key: &str) -> Option<Vec<u64>> {
    obj.get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_fast_variant_is_its_own_entry_with_the_server_name() {
        let entries = parse_model_entries(&json!({
            "data": [
                {"id": "grok-4.7", "name": "Grok 4.7", "contextWindows": [256000, 500000]},
                {"id": "grok-4.7-fast", "name": "Grok 4.7 Fast"},
                {"id": "grok-imagine-image", "hidden": true}
            ]
        }));
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["grok-4.7", "grok-4.7-fast", "grok-imagine-image"]);
        assert_eq!(entries[1].display_name(), "Grok 4.7 Fast");
        assert_eq!(entries[0].context_window, Some(256_000));
        assert_eq!(entries[0].context_windows, vec![256_000, 500_000]);
        assert!(!entries[2].visible_for_auth(true));
    }

    #[test]
    fn capabilities_menu_and_default_are_read_like_the_cli() {
        let entry = parse_model_value(&json!({
            "id": "grok-4.7",
            "capabilities": {
                "reasoning_effort": ["low", "medium", "high"],
                "default_reasoning_effort": "high"
            }
        }))
        .unwrap();
        assert_eq!(entry.supports_reasoning_effort, Some(true));
        assert_eq!(entry.reasoning_effort, Some(ReasoningEffort::High));
        let options = entry.options(ProviderFormat::Responses, 500_000);
        assert_eq!(options.efforts.len(), 3);
        assert_eq!(options.default_effort, Some(ReasoningEffort::High));
        assert_eq!(options.default_context_window, 500_000);
    }

    #[test]
    fn api_only_flag_hides_a_model_from_api_key_users() {
        let entry = parse_model_value(&json!({"id": "grok-x", "supportedInApi": false})).unwrap();
        assert!(entry.visible_for_auth(true));
        assert!(!entry.visible_for_auth(false));
    }

    #[test]
    fn context_choices_put_an_unlisted_default_first() {
        assert_eq!(
            context_window_choices(Some(128_000), &[256_000, 500_000]),
            Some((128_000, vec![128_000, 256_000, 500_000]))
        );
        assert_eq!(
            context_window_choices(None, &[256_000, 500_000]),
            Some((256_000, vec![256_000, 500_000]))
        );
        assert_eq!(context_window_choices(None, &[]), None);
    }

    #[test]
    fn default_models_json_menus_survive() {
        let grok_46 = known_model("grok-4.6").unwrap();
        let options = grok_46.options(ProviderFormat::Responses, 0);
        let values: Vec<ReasoningEffort> = options.efforts.iter().map(|o| o.value).collect();
        assert_eq!(
            values,
            [
                ReasoningEffort::Xhigh,
                ReasoningEffort::High,
                ReasoningEffort::Medium,
                ReasoningEffort::Low
            ]
        );
        assert_eq!(options.default_effort, Some(ReasoningEffort::High));
        assert_eq!(options.default_context_window, 500_000);
        assert_eq!(options.auto_compact_threshold_percent, 80);
        assert_eq!(options.efforts[0].label, "Extra High Effort");
    }

    #[test]
    fn unsupported_models_have_no_effort_and_unknown_choices_fall_back() {
        let fast = known_model("grok-4.7-fast").unwrap();
        let options = fast.options(ProviderFormat::Responses, 500_000);
        assert!(options.efforts.is_empty());
        assert_eq!(options.effort(Some(ReasoningEffort::High)), None);
        assert_eq!(options.context_window(Some(1)), 500_000);

        let grok_47 = known_model("grok-4.7").unwrap();
        let options = grok_47.options(ProviderFormat::Responses, 500_000);
        assert_eq!(
            options.effort(Some(ReasoningEffort::Xhigh)),
            Some(ReasoningEffort::Xhigh)
        );
        assert_eq!(
            options.effort(Some(ReasoningEffort::Max)),
            Some(ReasoningEffort::High)
        );
    }

    #[test]
    fn anthropic_models_support_effort_unless_told_otherwise() {
        let options = ModelInfo::bare("claude-x").options(ProviderFormat::Anthropic, 200_000);
        let values: Vec<ReasoningEffort> = options.efforts.iter().map(|o| o.value).collect();
        assert_eq!(values, FALLBACK_EFFORTS);
        assert_eq!(options.default_effort, None);
        let off = ModelInfo {
            supports_reasoning_effort: Some(false),
            ..ModelInfo::bare("claude-x")
        };
        assert!(off.options(ProviderFormat::Anthropic, 0).efforts.is_empty());
        assert!(
            ModelInfo::bare("gpt-4o")
                .options(ProviderFormat::OpenAi, 128_000)
                .efforts
                .is_empty()
        );
    }

    #[test]
    fn a_server_entry_keeps_its_fields_and_borrows_the_rest() {
        let mut entry = ModelInfo {
            name: Some("Grok 4.7 (server)".into()),
            ..ModelInfo::bare("grok-4.7")
        };
        entry.fill_from(&known_model("grok-4.7").unwrap());
        assert_eq!(entry.name.as_deref(), Some("Grok 4.7 (server)"));
        assert_eq!(entry.context_window, Some(500_000));
        assert!(!entry.reasoning_efforts.is_empty());
    }

    #[test]
    fn model_info_round_trips_through_toml() {
        let entry = known_model("grok-4.6").unwrap();
        #[derive(Serialize, Deserialize)]
        struct Doc {
            model_info: Vec<ModelInfo>,
        }
        let text = toml::to_string(&Doc {
            model_info: vec![entry.clone()],
        })
        .unwrap();
        let back: Doc = toml::from_str(&text).unwrap();
        assert_eq!(back.model_info[0], entry);
    }
}
