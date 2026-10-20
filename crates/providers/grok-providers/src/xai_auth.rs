//! xAI credentials for Official Grok.
//!
//! An OAuth session in the same `auth.json` that `xai-grok-login` writes
//! (`GROK_AUTH`, `$GROK_AUTH_PATH`, or `$GROK_HOME/auth.json`) takes priority
//! over an API key. The desktop window runs that login itself. This module only
//! reads the file, so the provider crate does not link the login stack.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

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

/// Redacted Official Grok session. Tokens never leave this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginProfile {
    pub signed_in: bool,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub expires_at: Option<String>,
    pub expired: bool,
    pub has_refresh: bool,
    pub auth_mode: Option<String>,
}

impl LoginProfile {
    pub fn signed_out() -> Self {
        Self {
            signed_in: false,
            email: None,
            display_name: None,
            expires_at: None,
            expired: false,
            has_refresh: false,
            auth_mode: None,
        }
    }
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
    if let Some(secret) = login.session.clone() {
        return Some((secret, XaiSecretSource::Session));
    }
    if let Some(secret) = nonempty(settings_secret) {
        return Some((secret, XaiSecretSource::Settings));
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

pub fn read_login_profile() -> LoginProfile {
    if let Ok(inline) = std::env::var(GROK_AUTH_ENV) {
        if let Some(profile) = profile_from_inline(&inline) {
            return profile;
        }
    }
    let Ok(text) = std::fs::read_to_string(auth_json_path()) else {
        return LoginProfile::signed_out();
    };
    profile_from_store_json(&text).unwrap_or_else(LoginProfile::signed_out)
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
    is_expired_at(auth.expires_at.as_deref(), auth.create_time.as_deref())
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

#[derive(Debug, Deserialize)]
struct StoredProfile {
    #[serde(default)]
    key: String,
    #[serde(default)]
    auth_mode: String,
    #[serde(default)]
    create_time: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

/// Who the current Grok session belongs to. The CLI sends these as
/// `x-userid` and `x-email` when it lists models through the chat proxy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionIdentity {
    pub user_id: Option<String>,
    pub email: Option<String>,
}

pub fn session_identity() -> SessionIdentity {
    let entries: Vec<(String, StoredProfile)> = match std::env::var(GROK_AUTH_ENV) {
        Ok(inline) => serde_json::from_str::<StoredProfile>(&inline)
            .map(|auth| vec![("inline".to_string(), auth)])
            .unwrap_or_default(),
        Err(_) => std::fs::read_to_string(auth_json_path())
            .ok()
            .and_then(|text| serde_json::from_str::<BTreeMap<String, StoredProfile>>(&text).ok())
            .map(|map| map.into_iter().collect())
            .unwrap_or_default(),
    };
    entries
        .into_iter()
        .filter(|(scope, auth)| {
            scope != API_KEY_SCOPE
                && auth.auth_mode != "api_key"
                && !is_legacy_mode(&auth.auth_mode)
                && !auth.key.trim().is_empty()
                && !is_expired_at(auth.expires_at.as_deref(), auth.create_time.as_deref())
        })
        .max_by(|left, right| left.1.create_time.cmp(&right.1.create_time))
        .map(|(_, auth)| SessionIdentity {
            user_id: nonempty(auth.user_id.as_deref()),
            email: nonempty(auth.email.as_deref()),
        })
        .unwrap_or_default()
}

fn profile_from_inline(json: &str) -> Option<LoginProfile> {
    let auth: StoredProfile = serde_json::from_str(json).ok()?;
    profile_from_entry("inline", &auth)
}

fn profile_from_store_json(json: &str) -> Option<LoginProfile> {
    let map: BTreeMap<String, StoredProfile> = serde_json::from_str(json).ok()?;
    let mut sessions: Vec<(String, LoginProfile)> = Vec::new();
    for (scope, auth) in &map {
        if let Some(profile) = profile_from_entry(scope, auth) {
            let stamp = auth.create_time.clone().unwrap_or_default();
            sessions.push((stamp, profile));
        }
    }
    sessions.sort_by(|left, right| right.0.cmp(&left.0));
    Some(
        sessions
            .into_iter()
            .next()
            .map(|(_, profile)| profile)
            .unwrap_or_else(LoginProfile::signed_out),
    )
}

fn profile_from_entry(scope: &str, auth: &StoredProfile) -> Option<LoginProfile> {
    if scope == API_KEY_SCOPE || auth.auth_mode == "api_key" || is_legacy_mode(&auth.auth_mode) {
        return None;
    }
    if auth.key.trim().is_empty() {
        return None;
    }
    let email = nonempty(auth.email.as_deref());
    let display_name = person_name(auth.first_name.as_deref(), auth.last_name.as_deref());
    let auth_mode = nonempty(Some(auth.auth_mode.as_str()));
    Some(LoginProfile {
        signed_in: true,
        email,
        display_name,
        expires_at: nonempty(auth.expires_at.as_deref()),
        expired: is_expired_at(auth.expires_at.as_deref(), auth.create_time.as_deref()),
        has_refresh: nonempty(auth.refresh_token.as_deref()).is_some(),
        auth_mode,
    })
}

fn person_name(first: Option<&str>, last: Option<&str>) -> Option<String> {
    let name = [first.unwrap_or(""), last.unwrap_or("")]
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    nonempty(Some(name.as_str()))
}

fn is_expired_at(expires_at: Option<&str>, create_time: Option<&str>) -> bool {
    let buffer = Duration::seconds(EARLY_INVALIDATION_SECS);
    if let Some(expires_at) = expires_at.and_then(parse_time) {
        return Utc::now() >= (expires_at - buffer);
    }
    if let Some(create_time) = create_time.and_then(parse_time) {
        let age = Utc::now().signed_duration_since(create_time);
        return age >= (Duration::days(TOKEN_TTL_DAYS) - buffer);
    }
    false
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
    fn oauth_session_beats_settings_key_and_env() {
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
            Some(("session-token".into(), XaiSecretSource::Session))
        );
    }

    #[test]
    fn settings_key_beats_env_when_no_session() {
        let login = LoginSnapshot {
            session: None,
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

    #[test]
    fn profile_reports_the_account_and_hides_tokens() {
        let json = r#"
        {
          "xai::api_key": {"key": "api-secret", "auth_mode": "api_key"},
          "https://auth.x.ai::client": {
            "key": "secret-token",
            "auth_mode": "oidc",
            "email": "ada@example.com",
            "first_name": "Ada",
            "last_name": "Lovelace",
            "refresh_token": "refresh-secret",
            "expires_at": "2099-01-01T00:00:00Z",
            "create_time": "2026-01-01T00:00:00Z"
          }
        }
        "#;
        let profile = profile_from_store_json(json).unwrap();
        assert!(profile.signed_in);
        assert!(!profile.expired);
        assert!(profile.has_refresh);
        assert_eq!(profile.email.as_deref(), Some("ada@example.com"));
        assert_eq!(profile.display_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(profile.auth_mode.as_deref(), Some("oidc"));
        let dumped = serde_json::to_string(&profile).unwrap();
        assert!(!dumped.contains("secret-token"));
        assert!(!dumped.contains("refresh-secret"));
        assert!(!dumped.contains("api-secret"));
    }

    #[test]
    fn expired_oauth_session_stays_visible_for_refresh() {
        let json = r#"
        {
          "https://auth.x.ai::client": {
            "key": "secret-token",
            "auth_mode": "oidc",
            "email": "ada@example.com",
            "expires_at": "2000-01-01T00:00:00Z",
            "refresh_token": "refresh-secret"
          }
        }
        "#;
        let profile = profile_from_store_json(json).unwrap();
        assert!(profile.signed_in);
        assert!(profile.expired);
        assert!(profile.has_refresh);
        let snapshot = snapshot_from_store_json(json).unwrap();
        assert!(snapshot.session.is_none());
    }
}
