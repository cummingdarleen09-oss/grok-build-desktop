//! API keys live in the OS keychain when it accepts them, and otherwise in a
//! mode `0600` file under the grok home. They are never written into git or
//! into `config.toml`.

use std::collections::BTreeMap;
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
}

impl SecretStore {
    pub fn open() -> Self {
        Self {
            file_path: grok_home().join("provider-secrets.json"),
        }
    }

    pub fn with_file(file_path: PathBuf) -> Self {
        Self { file_path }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<SecretLocation, String> {
        if secret.trim().is_empty() {
            return Err("API key is empty".into());
        }
        #[cfg(feature = "keyring")]
        if keyring_set(account, secret).is_ok() {
            let _ = self.delete_file_entry(account);
            return Ok(SecretLocation::Keyring);
        }
        self.set_file(account, secret)?;
        Ok(SecretLocation::File)
    }

    pub fn get(&self, account: &str) -> Result<Option<String>, String> {
        #[cfg(feature = "keyring")]
        if let Some(secret) = keyring_get(account)? {
            if !secret.trim().is_empty() {
                return Ok(Some(secret));
            }
        }
        self.get_file(account)
    }

    pub fn delete(&self, account: &str) -> Result<(), String> {
        #[cfg(feature = "keyring")]
        {
            let _ = keyring_delete(account);
        }
        self.delete_file_entry(account)
    }

    /// Key resolution for a provider: explicit override, then env vars, then the secret store.
    pub fn resolve(
        &self,
        provider: &ProviderConfig,
        explicit: Option<&str>,
    ) -> Result<Option<String>, String> {
        if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
            return Ok(Some(explicit.to_string()));
        }
        if provider.id == "xai" {
            let stored = self.get(&provider.id)?;
            let env = std::env::vars().collect();
            let login = crate::xai_auth::current_login();
            return Ok(
                crate::xai_auth::pick_xai_secret(stored.as_deref(), &env, &login)
                    .map(|(secret, _source)| secret),
            );
        }
        for name in provider.env_names() {
            if let Ok(value) = std::env::var(name) {
                let value = value.trim().to_string();
                if !value.is_empty() {
                    return Ok(Some(value));
                }
            }
        }
        self.get(&provider.id)
    }

    pub fn location(&self, account: &str) -> Result<Option<SecretLocation>, String> {
        #[cfg(feature = "keyring")]
        if keyring_get(account)?.is_some_and(|value| !value.trim().is_empty()) {
            return Ok(Some(SecretLocation::Keyring));
        }
        Ok(self.get_file(account)?.map(|_| SecretLocation::File))
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
        map.remove(account);
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
            Ok(text) => serde_json::from_str(&text).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write_file_map(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let json = serde_json::to_string_pretty(map).map_err(|error| error.to_string())?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.file_path)
            .map_err(|error| error.to_string())?;
        file.write_all(json.as_bytes())
            .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&self.file_path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
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
        assert_eq!(location, SecretLocation::File);
        assert_eq!(
            store.get("deepseek").unwrap().as_deref(),
            Some("sk-test-value")
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
    }
}
