//! API keys live in the OS keychain when it accepts them, and otherwise in a
//! mode `0600` file under the grok home. They are never written into git or
//! into `config.toml`.
//!
//! A keychain write only counts after the key reads back. The accounts that
//! made it into the keychain are listed (names only) in a small index file, so
//! status checks never touch the keychain. On macOS each keychain read can
//! raise an access prompt, which would otherwise block startup.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config_merge::grok_home;
use crate::types::ProviderConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretLocation {
    Keyring,
    File,
}

#[derive(Debug, Clone)]
pub struct SecretStore {
    file_path: PathBuf,
    index_path: PathBuf,
}

impl SecretStore {
    pub fn open() -> Self {
        Self::with_file(grok_home().join("provider-secrets.json"))
    }

    pub fn with_file(file_path: PathBuf) -> Self {
        let index_path = file_path.with_file_name(format!(
            "{}.keyring-index.json",
            file_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("provider-secrets")
        ));
        Self {
            file_path,
            index_path,
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<SecretLocation, String> {
        let secret = secret.trim();
        if secret.is_empty() {
            return Err("API key is empty".into());
        }
        #[cfg(feature = "keyring")]
        if keyring_set(account, secret).is_ok()
            && keyring_get(account).ok().flatten().as_deref() == Some(secret)
        {
            self.set_indexed(account, true)?;
            let _ = self.delete_file_entry(account);
            return Ok(SecretLocation::Keyring);
        }
        self.set_file(account, secret)?;
        self.forget_keyring(account);
        Ok(SecretLocation::File)
    }

    pub fn get(&self, account: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "keyring")]
        if self.indexed(account)
            && let Some(secret) = keyring_get(account)?
            && !secret.trim().is_empty()
        {
            return Ok(Some(secret));
        }
        Ok(self
            .get_file(account)?
            .filter(|secret| !secret.trim().is_empty()))
    }

    pub fn delete(&self, account: &str) -> Result<(), String> {
        self.forget_keyring(account);
        self.delete_file_entry(account)
    }

    /// Key resolution for a provider: explicit override, then env vars, then the secret store.
    pub fn resolve(
        &self,
        provider: &ProviderConfig,
        explicit: Option<&str>,
    ) -> Result<Option<String>, String> {
        Ok(self
            .resolve_with_source(provider, explicit)?
            .map(|(secret, _)| secret))
    }

    /// Like [`Self::resolve`], also reporting where the key came from. Official
    /// Grok needs this because an OAuth session is sent to a different host
    /// than an API key.
    pub fn resolve_with_source(
        &self,
        provider: &ProviderConfig,
        explicit: Option<&str>,
    ) -> Result<Option<(String, crate::xai_auth::XaiSecretSource)>, String> {
        use crate::xai_auth::XaiSecretSource;
        if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
            return Ok(Some((explicit.to_string(), XaiSecretSource::Settings)));
        }
        if provider.id == "xai" {
            let stored = self.get(&provider.id)?;
            let env = std::env::vars().collect();
            let login = crate::xai_auth::current_login();
            return Ok(crate::xai_auth::pick_xai_secret(
                stored.as_deref(),
                &env,
                &login,
            ));
        }
        for name in provider.env_names() {
            if let Ok(value) = std::env::var(name) {
                let value = value.trim().to_string();
                if !value.is_empty() {
                    return Ok(Some((value, XaiSecretSource::Env)));
                }
            }
        }
        Ok(self
            .get(&provider.id)?
            .map(|secret| (secret, XaiSecretSource::Settings)))
    }

    /// Where a saved key lives. This never reads the keychain.
    pub fn location(&self, account: &str) -> Result<Option<SecretLocation>, String> {
        #[cfg(feature = "keyring")]
        if self.indexed(account) {
            return Ok(Some(SecretLocation::Keyring));
        }
        Ok(self
            .get_file(account)?
            .filter(|secret| !secret.trim().is_empty())
            .map(|_| SecretLocation::File))
    }

    fn forget_keyring(&self, account: &str) {
        #[cfg(feature = "keyring")]
        if self.indexed(account) {
            let _ = keyring_delete(account);
        }
        let _ = self.set_indexed(account, false);
    }

    #[cfg_attr(not(feature = "keyring"), allow(dead_code))]
    fn indexed(&self, account: &str) -> bool {
        self.read_index().contains(account)
    }

    fn read_index(&self) -> BTreeSet<String> {
        std::fs::read_to_string(&self.index_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn set_indexed(&self, account: &str, present: bool) -> Result<(), String> {
        let mut index = self.read_index();
        let changed = if present {
            index.insert(account.to_string())
        } else {
            index.remove(account)
        };
        if !changed {
            return Ok(());
        }
        if index.is_empty() {
            return match std::fs::remove_file(&self.index_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            };
        }
        let json = serde_json::to_string_pretty(&index).map_err(|error| error.to_string())?;
        write_private(&self.index_path, &json)
    }

    fn set_file(&self, account: &str, secret: &str) -> Result<(), String> {
        let mut map = self.read_file_map()?;
        map.insert(account.to_string(), secret.to_string());
        self.write_file_map(&map)
    }

    fn get_file(&self, account: &str) -> Result<Option<String>, String> {
        let map = self.read_file_map()?;
        Ok(map.get(account).cloned())
    }

    fn delete_file_entry(&self, account: &str) -> Result<(), String> {
        let mut map = self.read_file_map()?;
        if map.remove(account).is_none() {
            return Ok(());
        }
        if map.is_empty() {
            if self.file_path.exists() {
                std::fs::remove_file(&self.file_path).map_err(|error| error.to_string())?;
            }
            return Ok(());
        }
        self.write_file_map(&map)
    }

    fn read_file_map(&self) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read_to_string(&self.file_path) {
            Ok(text) if text.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                format!("{} is not valid JSON: {error}", self.file_path.display())
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write_file_map(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        let json = serde_json::to_string_pretty(map).map_err(|error| error.to_string())?;
        write_private(&self.file_path, &json)
    }
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(feature = "keyring")]
fn keyring_entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("grok-desktop", account).map_err(|error| error.to_string())
}

#[cfg(feature = "keyring")]
fn keyring_set(account: &str, secret: &str) -> Result<(), String> {
    keyring_entry(account)?
        .set_password(secret)
        .map_err(|error| error.to_string())
}

#[cfg(feature = "keyring")]
fn keyring_get(account: &str) -> Result<Option<String>, String> {
    match keyring_entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => {
            tracing::debug!(%error, "keyring read failed; falling back to the secrets file");
            Ok(None)
        }
    }
}

#[cfg(feature = "keyring")]
fn keyring_delete(account: &str) -> Result<(), String> {
    match keyring_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::with_file(dir.path().join("secrets.json"));
        let location = store.set("deepseek", "sk-test-value").unwrap();
        // Linux test builds have no keychain backend (keyring falls back to a
        // non-persistent mock), so the verified write must land in the file.
        assert_eq!(location, SecretLocation::File);
        assert_eq!(
            store.get("deepseek").unwrap().as_deref(),
            Some("sk-test-value")
        );
        assert_eq!(
            store.location("deepseek").unwrap(),
            Some(SecretLocation::File)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.file_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete("deepseek").unwrap();
        assert_eq!(store.get("deepseek").unwrap(), None);
        assert_eq!(store.location("deepseek").unwrap(), None);
    }

    #[test]
    fn saved_key_survives_a_new_store_instance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        SecretStore::with_file(path.clone())
            .set("relay", "  sk-relay  ")
            .unwrap();
        let reopened = SecretStore::with_file(path);
        assert_eq!(reopened.get("relay").unwrap().as_deref(), Some("sk-relay"));
    }
}
