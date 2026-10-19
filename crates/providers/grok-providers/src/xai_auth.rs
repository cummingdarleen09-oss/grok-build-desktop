//! xAI credentials for the desktop preset.
//!
//! Resolution matches the upstream seam in `xai-grok-shell`'s `resolve_credentials`:
//! an explicit key, then a `grok login` session, then `XAI_API_KEY`.
//! The session is read from the same `auth.json` document `xai-grok-login` writes
//! (`GROK_AUTH`, `$GROK_AUTH_PATH`, or `$GROK_HOME/auth.json`). Linking that crate
//! would pull the sampler and the tool stack into the desktop binary, so this
//! module reads the file format directly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::config_merge::grok_home;

pub const XAI_API_KEY_ENV: &str = "XAI_API_KEY";
pub const LEGACY_XAI_API_KEY_ENV: &str = "GROK_CODE_XAI_API_KEY";
/// Scope `grok login --api-key` uses. Same string as `xai_grok_login::model::API_KEY_SCOPE`.
pub const API_KEY_SCOPE: &str = "xai::api_key";

const TOKEN_TTL_DAYS: i64 = 30;
const EARLY_INVALIDATION_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XaiSecretSource {
    Settings,
    Session,
    Env,
    LoginApiKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginSnapshot {
    pub session: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StoredAuth {
    #[serde(default)]
    key: String,
    #[serde(default)]
    auth_mode: String,
    #[serde(default)]
    create_time: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

pub fn pick_xai_secret(
    settings_secret: Option<&str>,
    env: &BTreeMap<String, String>,
    login: &LoginSnapshot,
) -> Option<(String, XaiSecretSource)> {
    if let Some(secret) = nonempty(settings_secret) {
        return Some((secret, XaiSecretSource::Settings));
    }
    if let Some(secret) = login.session.clone() {
        return Some((secret, XaiSecretSource::Session));
    }
    if let Some(secret) = env_key(env) {
        return Some((secret, XaiSecretSource::Env));
    }
    login
        .api_key
        .clone()
        .map(|secret| (secret, XaiSecretSource::LoginApiKey))
}

pub fn env_key(env: &BTreeMap<String, String>) -> Option<String> {
    nonempty(env.get(XAI_API_KEY_ENV).map(String::as_str))
        .or_else(|| nonempty(env.get(LEGACY_XAI_API_KEY_ENV).map(String::as_str)))
}

pub fn current_login() -> LoginSnapshot {
    if let Some(inline) = std::env::var(GROK_AUTH_ENV).ok() {
        if let Some(snapshot) = snapshot_from_inline(&inline) {
            return snapshot;
        }
    }
    load_login_file(&auth_json_path())
}

pub fn has_login_session() -> bool {
    current_login().session.is_some()
}

const GROK_AUTH_ENV: &str = "GROK_AUTH";

pub fn auth_json_path() -> PathBuf {
    if let Some(override_path) = std::env::var_os("GROK_AUTH_PATH") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    grok_home().join("auth.json")
}

pub fn load_login_file(path: &Path) -> LoginSnapshot {
    let Ok(text) = std::fs::read_to_string(path) else {
        return LoginSnapshot::default();
    };
    snapshot_from_store_json(&text).unwrap_or_default()
}

fn snapshot_from_inline(json: &str) -> Option<LoginSnapshot> {
    let auth: StoredAuth = serde_json::from_str(json).ok()?;
    let mut snapshot = LoginSnapshot::default();
    push_auth(&mut snapshot, "inline", &auth);
    (snapshot.session.is_some() || snapshot.api_key.is_some()).then_some(snapshot)
}

fn snapshot_from_store_json(json: &str) -> Option<LoginSnapshot> {
    let map: BTreeMap<String, StoredAuth> = serde_json::from_str(json).ok()?;
    let mut snapshot = LoginSnapshot::default();
    let mut sessions: Vec<(String, String)> = Vec::new();
    for (scope, auth) in &map {
        if scope == API_KEY_SCOPE {
            if let Some(key) = usable_key(auth) {
                snapshot.api_key = Some(key);
            }
            continue;
        }
        if let Some(key) = usable_key(auth) {
            let stamp = auth.create_time.clone().unwrap_or_default();
            sessions.push((stamp, key));
        }
    }
    sessions.sort_by(|left, right| right.0.cmp(&left.0));
    snapshot.session = sessions.into_iter().next().map(|(_, key)| key);
    Some(snapshot)
}

fn push_auth(snapshot: &mut LoginSnapshot, scope: &str, auth: &StoredAuth) {
    let Some(key) = usable_key(auth) else {
        return;
    };
    if scope == API_KEY_SCOPE || auth.auth_mode == "api_key" {
        snapshot.api_key = Some(key);
    } else {
        snapshot.session = Some(key);
    }
}

fn usable_key(auth: &StoredAuth) -> Option<String> {
    let key = auth.key.trim();
    if key.is_empty() || is_legacy_mode(&auth.auth_mode) || is_expired(auth) {
        return None;
    }
    Some(key.to_string())
}

fn is_legacy_mode(mode: &str) -> bool {
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "web_login" | "weblogin" | "grok"
    )
}

fn is_expired(auth: &StoredAuth) -> bool {
    let buffer = Duration::seconds(EARLY_INVALIDATION_SECS);
    if let Some(expires_at) = auth.expires_at.as_deref().and_then(parse_time) {
        return Utc::now() >= (expires_at - buffer);
    }
    if let Some(create_time) = auth.create_time.as_deref().and_then(parse_time) {
        let age = Utc::now().signed_duration_since(create_time);
        return age >= (Duration::days(TOKEN_TTL_DAYS) - buffer);
    }
    false
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn settings_key_beats_session_and_env() {
        let login = LoginSnapshot {
            session: Some("session-token".into()),
            api_key: Some("login-api-key".into()),
        };
        let picked = pick_xai_secret(
            Some("settings-key"),
            &env(&[(XAI_API_KEY_ENV, "env-key")]),
            &login,
        );
        assert_eq!(
            picked,
            Some(("settings-key".into(), XaiSecretSource::Settings))
        );
    }

    #[test]
    fn session_beats_env_key() {
        let login = LoginSnapshot {
            session: Some("session-token".into()),
            api_key: None,
        };
        let picked = pick_xai_secret(None, &env(&[(XAI_API_KEY_ENV, "env-key")]), &login);
        assert_eq!(
            picked,
            Some(("session-token".into(), XaiSecretSource::Session))
        );
    }

    #[test]
    fn env_key_beats_legacy_and_login_api_key() {
        let login = LoginSnapshot {
            session: None,
            api_key: Some("login-api-key".into()),
        };
        let picked = pick_xai_secret(
            None,
            &env(&[
                (XAI_API_KEY_ENV, "new-key"),
                (LEGACY_XAI_API_KEY_ENV, "old-key"),
            ]),
            &login,
        );
        assert_eq!(picked, Some(("new-key".into(), XaiSecretSource::Env)));
    }

    #[test]
    fn legacy_env_and_login_api_key_are_fallbacks() {
        let login = LoginSnapshot {
            session: None,
            api_key: Some("login-api-key".into()),
        };
        assert_eq!(
            pick_xai_secret(None, &env(&[(LEGACY_XAI_API_KEY_ENV, "old-key")]), &login),
            Some(("old-key".into(), XaiSecretSource::Env))
        );
        assert_eq!(
            pick_xai_secret(None, &BTreeMap::new(), &login),
            Some(("login-api-key".into(), XaiSecretSource::LoginApiKey))
        );
    }

    #[test]
    fn auth_json_skips_expired_and_legacy_sessions() {
        let json = r#"
        {
          "xai::api_key": {"key": "stored-api", "auth_mode": "api_key"},
          "https://auth.x.ai::client": {
            "key": "fresh-session",
            "auth_mode": "oidc",
            "create_time": "2099-01-01T00:00:00Z"
          },
          "old": {
            "key": "stale-session",
            "auth_mode": "oidc",
            "expires_at": "2000-01-01T00:00:00Z"
          },
          "legacy": {"key": "legacy-token", "auth_mode": "web_login"}
        }
        "#;
        let snapshot = snapshot_from_store_json(json).unwrap();
        assert_eq!(snapshot.session.as_deref(), Some("fresh-session"));
        assert_eq!(snapshot.api_key.as_deref(), Some("stored-api"));
    }
}
