// auth_store.rs — JSON-based credential store at ~/.coven-code/auth.json.
//
// Stores API keys and OAuth tokens for providers so users don't have to rely
// solely on environment variables.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A stored credential for a provider.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StoredCredential {
    #[serde(rename = "api")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuthToken {
        access: String,
        refresh: String,
        expires: u64,
    },
}

impl std::fmt::Debug for StoredCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("key", &"[REDACTED]")
                .finish(),
            Self::OAuthToken { expires, .. } => f
                .debug_struct("OAuthToken")
                .field("access", &"[REDACTED]")
                .field("refresh", &"[REDACTED]")
                .field("expires", expires)
                .finish(),
        }
    }
}

/// Persistent credential store backed by `~/.coven-code/auth.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    pub credentials: HashMap<String, StoredCredential>,
}

impl AuthStore {
    /// Path to the auth store file.
    pub fn path() -> PathBuf {
        crate::config::config_home().join("auth.json")
    }

    /// Load the store from disk (returns default if missing or invalid).
    pub fn load() -> Self {
        let path = Self::path();
        if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        }
    }

    /// Persist the store to disk (best-effort) using an atomic private write.
    ///
    /// Writes go to a sibling tempfile and then `fs::rename` swaps it into
    /// place. This avoids torn writes (a partially-written `auth.json` if
    /// the process is killed mid-write). Concurrent writers remain
    /// last-writer-wins; this store does not merge independent updates.
    ///
    /// This compatibility wrapper cannot report persistence failures. Callers
    /// that need the error must use [`Self::save_to`].
    pub fn save(&self) {
        let _ = self.save_to(&Self::path());
    }

    /// Atomic save to an arbitrary path. Returns an io::Error on failure so
    /// tests can assert behaviour. The public `save()` wrapper keeps the
    /// best-effort void return for backwards compatibility.
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::secret_file::write_secret_file_sync(path, json)
    }

    /// Store a credential for the given provider (persists immediately).
    pub fn set(&mut self, provider_id: &str, cred: StoredCredential) {
        self.credentials.insert(provider_id.to_string(), cred);
        self.save();
    }

    /// Get the stored credential for a provider.
    pub fn get(&self, provider_id: &str) -> Option<&StoredCredential> {
        self.credentials.get(provider_id)
    }

    /// Remove the credential for a provider (persists immediately).
    pub fn remove(&mut self, provider_id: &str) {
        self.credentials.remove(provider_id);
        self.save();
    }

    /// Get the API key for a provider, checking stored credentials first then
    /// falling back to the relevant environment variable.
    pub fn api_key_for(&self, provider_id: &str) -> Option<String> {
        // Check stored credentials first
        if let Some(stored) = self.get(provider_id) {
            match stored {
                StoredCredential::ApiKey { key } if !key.is_empty() => {
                    return Some(key.clone());
                }
                StoredCredential::ApiKey { .. } => {}
                _ => {}
            }
        }
        // Fall back to environment variable.
        let env_var = match provider_id {
            "anthropic" => "ANTHROPIC_API_KEY",
            "openai" | "codex" | "openai-codex" => "OPENAI_API_KEY",
            _ => return None,
        };
        if env_var.is_empty() {
            None
        } else {
            std::env::var(env_var).ok().filter(|k| !k.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthStore, StoredCredential};

    #[test]
    fn credential_debug_omits_secrets_but_keeps_variant_and_expiry() {
        let api = StoredCredential::ApiKey {
            key: "canary-api-key".to_string(),
        };
        let oauth = StoredCredential::OAuthToken {
            access: "canary-access-token".to_string(),
            refresh: "canary-refresh-token".to_string(),
            expires: 42,
        };
        let mut store = AuthStore::default();
        store.credentials.insert("api".to_string(), api.clone());
        store.credentials.insert("oauth".to_string(), oauth.clone());

        for rendered in [
            format!("{api:?}"),
            format!("{oauth:?}"),
            format!("{store:?}"),
        ] {
            assert!(!rendered.contains("canary-api-key"), "{rendered}");
            assert!(!rendered.contains("canary-access-token"), "{rendered}");
            assert!(!rendered.contains("canary-refresh-token"), "{rendered}");
        }
        assert!(format!("{api:?}").contains("ApiKey"));
        assert!(format!("{oauth:?}").contains("42"));
        assert_eq!(serde_json::to_value(&api).unwrap()["key"], "canary-api-key");
        assert_eq!(
            serde_json::to_value(&oauth).unwrap()["access"],
            "canary-access-token"
        );
    }

    #[test]
    fn api_key_for_regular_provider_uses_stored_key() {
        let mut store = AuthStore::default();
        store.credentials.insert(
            "anthropic".to_string(),
            StoredCredential::ApiKey {
                key: "sk-ant-key".to_string(),
            },
        );

        assert_eq!(
            store.api_key_for("anthropic").as_deref(),
            Some("sk-ant-key")
        );
    }

    /// Atomic save: a concurrent racer that creates the file mid-flight
    /// must never leave us with a truncated `auth.json`. We test by writing
    /// 50 stores in parallel to the same path and asserting the final
    /// contents are a parseable AuthStore.
    #[test]
    fn save_to_is_atomic_under_concurrent_writers() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("auth.json"));

        let handles: Vec<_> = (0..50)
            .map(|i| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    let mut store = AuthStore::default();
                    store.credentials.insert(
                        format!("provider-{i}"),
                        StoredCredential::ApiKey {
                            key: format!("key-{i}"),
                        },
                    );
                    store.save_to(&path).expect("save_to should succeed");
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread panicked");
        }

        // The file must end up as a parseable AuthStore with exactly one
        // credential (whichever writer rename'd last). Crucially, partial
        // truncated JSON would fail to parse.
        let body = std::fs::read_to_string(&*path).expect("file must exist");
        let parsed: AuthStore = serde_json::from_str(&body)
            .expect("final auth.json must be a valid AuthStore (not torn)");
        assert_eq!(
            parsed.credentials.len(),
            1,
            "exactly one writer's entry should survive"
        );
    }

    /// The credential file must not be world/group-readable: it holds API
    /// keys and OAuth tokens. After an atomic save the destination file should
    /// be mode 0600 on Unix.
    #[cfg(unix)]
    #[test]
    fn save_to_restricts_permissions_to_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("auth.json");
        let mut store = AuthStore::default();
        store.credentials.insert(
            "anthropic".to_string(),
            StoredCredential::ApiKey {
                key: "sk-ant-secret".to_string(),
            },
        );
        store.save_to(&path).expect("save_to should succeed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "auth.json must be owner-only, got {mode:o}");
        let parent_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            parent_mode, 0o755,
            "AuthStore must not chmod the broad config-home parent"
        );
    }

    #[test]
    fn save_to_does_not_leave_tempfile_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = AuthStore::default();
        store.save_to(&path).expect("save_to should succeed");

        // The destination file is the only entry; no `.auth.json.tmp.*`
        // residue should remain after a successful rename.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "auth.json")
            .collect();
        assert!(
            leftovers.is_empty(),
            "expected no tempfile residue, found {leftovers:?}"
        );
    }
}
