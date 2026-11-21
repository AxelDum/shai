use crate::tools::McpClient;
use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthError, CredentialRefreshGuard, CredentialStore, StoredCredentials,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Weak},
};

use super::{HttpClient, SseClient, StdioClient};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    /// Unix timestamp (seconds since epoch) when the token expires
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpConfig {
    #[serde(rename = "stdio")]
    Stdio { command: String, args: Vec<String> },
    #[serde(rename = "http")]
    Http {
        url: String,
        #[serde(flatten)]
        auth: Option<OAuthToken>,
    },
    #[serde(rename = "sse")]
    Sse { url: String },
}

impl OAuthToken {
    /// Check if the token is expired or will expire within the next 60 seconds
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            // Consider expired if it expires within the next 60 seconds (safety margin)
            expires_at <= now + 60
        } else {
            // If no expiration time, assume it's still valid
            false
        }
    }
}

#[derive(Clone)]
pub struct McpCredentialStore {
    backend: McpCredentialBackend,
    refresh_guard: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
enum McpCredentialBackend {
    File(PathBuf),
    Memory(Arc<tokio::sync::RwLock<Option<StoredCredentials>>>),
}

impl McpCredentialStore {
    pub fn in_memory() -> Self {
        Self {
            backend: McpCredentialBackend::Memory(Arc::new(tokio::sync::RwLock::new(None))),
            refresh_guard: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn for_agent(
        agent_name: &str,
        mcp_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self::from_backend(Self::for_agent_path(
            agent_name, mcp_name,
        )?))
    }

    pub async fn has_saved_credentials(&self) -> Result<bool, AuthError> {
        Ok(self.load().await?.is_some())
    }

    pub fn delete_agent(agent_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let directory = Self::agent_directory(agent_name)?;
        match std::fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Delete stored credentials of this agent's MCP servers that are not listed.
    pub fn retain_agent<'a>(
        agent_name: &str,
        mcp_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Self::retain_in(&Self::agent_directory(agent_name)?, mcp_names)
    }

    fn retain_in<'a>(
        directory: &Path,
        mcp_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let kept: std::collections::HashSet<String> =
            mcp_names.into_iter().map(Self::hash).collect();
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            let Some(hash) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| {
                    name.strip_suffix(".json")
                        .or_else(|| name.strip_suffix(".lock"))
                })
            else {
                continue;
            };
            let is_credential_file =
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
            if is_credential_file && !kept.contains(hash) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    fn for_agent_path(
        agent_name: &str,
        mcp_name: &str,
    ) -> Result<McpCredentialBackend, Box<dyn std::error::Error + Send + Sync>> {
        Ok(McpCredentialBackend::File(
            Self::agent_directory(agent_name)?.join(format!("{}.json", Self::hash(mcp_name))),
        ))
    }

    fn agent_directory(
        agent_name: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self::config_directory()?
            .join("mcp-credentials")
            .join(Self::hash(agent_name)))
    }

    fn config_directory() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| {
                dirs::home_dir()
                    .map(|home| home.join(".config"))
                    .ok_or("Could not find home directory")
            })?;
        Ok(config_dir.join("shai"))
    }

    fn from_backend(backend: McpCredentialBackend) -> Self {
        let refresh_guard = match &backend {
            McpCredentialBackend::File(path) => Self::shared_refresh_guard(path),
            McpCredentialBackend::Memory(_) => Arc::new(tokio::sync::Mutex::new(())),
        };
        Self {
            backend,
            refresh_guard,
        }
    }

    /// Stores for the same credential file share one refresh guard, so
    /// sessions in this process don't refresh the same token concurrently.
    fn shared_refresh_guard(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        type Guards = std::sync::Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>;
        static GUARDS: LazyLock<Guards> = LazyLock::new(Default::default);
        let mut guards = GUARDS.lock().unwrap();
        guards.retain(|_, guard| guard.strong_count() > 0);
        if let Some(guard) = guards.get(path).and_then(Weak::upgrade) {
            return guard;
        }
        let guard = Arc::new(tokio::sync::Mutex::new(()));
        guards.insert(path.to_path_buf(), Arc::downgrade(&guard));
        guard
    }

    /// Locks `{hash}.lock` next to the credential file, so other processes
    /// don't refresh the same token concurrently.
    async fn lock_file(path: &Path) -> Result<std::fs::File, AuthError> {
        let lock_path = path.with_extension("lock");
        tokio::task::spawn_blocking(move || {
            let lock = || -> std::io::Result<std::fs::File> {
                if let Some(directory) = lock_path.parent() {
                    let mut builder = std::fs::DirBuilder::new();
                    builder.recursive(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::DirBuilderExt;
                        builder.mode(0o700);
                    }
                    builder.create(directory)?;
                }
                let mut options = std::fs::OpenOptions::new();
                options.create(true).truncate(false).write(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let file = options.open(&lock_path)?;
                file.lock()?;
                Ok(file)
            };
            lock().map_err(|error| {
                AuthError::CredentialStoreError(format!(
                    "failed to lock OAuth credentials at {}: {}",
                    lock_path.display(),
                    error
                ))
            })
        })
        .await
        .map_err(|error| AuthError::CredentialStoreError(error.to_string()))?
    }

    fn hash(value: &str) -> String {
        Sha256::digest(value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn from_path(path: PathBuf) -> Self {
        Self::from_backend(McpCredentialBackend::File(path))
    }

    fn load_file(path: &Path) -> Result<Option<StoredCredentials>, AuthError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(AuthError::CredentialStoreError(format!(
                    "failed to read OAuth credentials from {}: {}",
                    path.display(),
                    error
                )));
            }
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            AuthError::CredentialStoreError(format!(
                "failed to parse OAuth credentials from {}: {}",
                path.display(),
                error
            ))
        })
    }

    fn save_file(path: &Path, credentials: &StoredCredentials) -> Result<(), AuthError> {
        let directory = path.parent().ok_or_else(|| {
            AuthError::CredentialStoreError("OAuth credential file has no parent directory".into())
        })?;
        std::fs::create_dir_all(directory).map_err(|error| {
            AuthError::CredentialStoreError(format!(
                "failed to create OAuth credential directory {}: {}",
                directory.display(),
                error
            ))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    AuthError::CredentialStoreError(format!(
                        "failed to secure OAuth credential directory {}: {}",
                        directory.display(),
                        error
                    ))
                },
            )?;
        }

        let serialized = serde_json::to_vec_pretty(credentials)
            .map_err(|error| AuthError::CredentialStoreError(error.to_string()))?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));

        let write_result = (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&serialized)?;
            file.sync_all()?;
            drop(file);
            #[cfg(windows)]
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            std::fs::rename(&temporary, path)
        })();

        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary);
            return Err(AuthError::CredentialStoreError(format!(
                "failed to save OAuth credentials to {}: {}",
                path.display(),
                error
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for McpCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match &self.backend {
            McpCredentialBackend::File(path) => Self::load_file(path),
            McpCredentialBackend::Memory(credentials) => Ok(credentials.read().await.clone()),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        match &self.backend {
            McpCredentialBackend::File(path) => Self::save_file(path, &credentials),
            McpCredentialBackend::Memory(store) => {
                *store.write().await = Some(credentials);
                Ok(())
            }
        }
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match &self.backend {
            McpCredentialBackend::File(path) => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(AuthError::CredentialStoreError(format!(
                    "failed to clear OAuth credentials at {}: {}",
                    path.display(),
                    error
                ))),
            },
            McpCredentialBackend::Memory(credentials) => {
                *credentials.write().await = None;
                Ok(())
            }
        }
    }

    async fn on_refresh_token_rejected(
        &self,
        credentials: &StoredCredentials,
    ) -> Result<(), AuthError> {
        // Keep credentials that another session has already replaced
        let token = |credentials: &StoredCredentials| {
            serde_json::to_value(&credentials.token_response).ok()
        };
        match self.load().await? {
            Some(stored) if token(&stored) == token(credentials) => self.clear().await,
            _ => Ok(()),
        }
    }

    async fn acquire_refresh_guard(&self) -> Result<Option<CredentialRefreshGuard>, AuthError> {
        let guard = self.refresh_guard.clone().lock_owned().await;
        let lock_file = match &self.backend {
            McpCredentialBackend::File(path) => Some(Self::lock_file(path).await?),
            McpCredentialBackend::Memory(_) => None,
        };
        Ok(Some(CredentialRefreshGuard::new((guard, lock_file))))
    }
}

/// Factory function to create an MCP client from configuration
pub fn create_mcp_client(config: McpConfig) -> Box<dyn McpClient> {
    match config {
        McpConfig::Http { url, auth } => {
            let bearer_token = auth.map(|t| t.access_token);
            Box::new(HttpClient::new_with_auth(url, bearer_token))
        }
        McpConfig::Stdio { command, args } => Box::new(StdioClient::new(command, args)),
        McpConfig::Sse { url } => Box::new(SseClient::new(url)),
    }
}

pub(crate) fn create_mcp_client_with_store(
    config: McpConfig,
    credential_store: McpCredentialStore,
) -> Box<dyn McpClient> {
    match config {
        McpConfig::Http { url, auth } => Box::new(HttpClient::with_credential_store(
            url,
            auth,
            credential_store,
        )),
        McpConfig::Stdio { command, args } => Box::new(StdioClient::new(command, args)),
        McpConfig::Sse { url } => Box::new(SseClient::new(url)),
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    use rmcp::transport::auth::OAuthTokenResponse;
    use serde_json::json;

    fn credentials(access_token: &str, refresh_token: &str) -> StoredCredentials {
        let response: OAuthTokenResponse = serde_json::from_value(json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "refresh_token": refresh_token,
            "expires_in": 3600,
            "scope": "all"
        }))
        .unwrap();
        StoredCredentials::new(
            "client-id".into(),
            Some(response),
            vec!["all".into()],
            Some(123),
        )
        .with_issuer(Some("https://issuer.example".into()))
    }

    #[tokio::test]
    async fn credential_file_round_trip_is_user_only() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("nested").join("oauth.json");
        let store = McpCredentialStore::from_path(path.clone());
        store.save(credentials("access", "refresh")).await.unwrap();

        let loaded = store.load().await.unwrap().unwrap();
        assert_eq!(loaded.client_id, "client-id");
        assert!(store.has_saved_credentials().await.unwrap());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }

        store
            .save(credentials("access-2", "refresh-2"))
            .await
            .unwrap();
        let refreshed = store.load().await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(refreshed.token_response.unwrap()).unwrap()["access_token"],
            "access-2"
        );
        store.clear().await.unwrap();
        assert!(store.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejected_refresh_credentials_are_cleared() {
        let directory = tempfile::TempDir::new().unwrap();
        let store = McpCredentialStore::from_path(directory.path().join("oauth.json"));
        let credentials = credentials("access", "refresh");
        store.save(credentials.clone()).await.unwrap();
        store.on_refresh_token_rejected(&credentials).await.unwrap();
        assert!(store.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejected_refresh_keeps_rotated_credentials() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("oauth.json");
        let store = McpCredentialStore::from_path(path.clone());
        let rejected = credentials("access", "refresh");
        store.save(rejected.clone()).await.unwrap();
        // Another session refreshes and saves a rotated token first
        McpCredentialStore::from_path(path)
            .save(credentials("access-2", "refresh-2"))
            .await
            .unwrap();

        store.on_refresh_token_rejected(&rejected).await.unwrap();

        let kept = store
            .load()
            .await
            .unwrap()
            .expect("rotated credentials were cleared");
        assert_eq!(
            serde_json::to_value(kept.token_response.unwrap()).unwrap()["refresh_token"],
            "refresh-2"
        );
    }

    #[tokio::test]
    async fn refresh_guard_is_shared_by_credential_file() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("oauth.json");
        let first = McpCredentialStore::from_path(path.clone());
        let second = McpCredentialStore::from_path(path.clone());

        let guard = first.acquire_refresh_guard().await.unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                second.acquire_refresh_guard()
            )
            .await
            .is_err(),
            "two stores for the same file refreshed concurrently"
        );
        // Other processes are excluded through the lock file
        let lock_file = std::fs::OpenOptions::new()
            .write(true)
            .open(path.with_extension("lock"))
            .expect("no lock file while refreshing");
        assert!(lock_file.try_lock().is_err());

        drop(guard);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            second.acquire_refresh_guard(),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn removed_mcp_credentials_are_deleted() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = |name: &str| {
            directory
                .path()
                .join(format!("{}.json", McpCredentialStore::hash(name)))
        };
        for name in ["kept", "removed"] {
            McpCredentialStore::from_path(path(name))
                .save(credentials("access", "refresh"))
                .await
                .unwrap();
        }
        std::fs::write(directory.path().join("unrelated.json"), "{}").unwrap();
        std::fs::write(path("removed").with_extension("lock"), "").unwrap();

        McpCredentialStore::retain_in(directory.path(), ["kept"]).unwrap();

        assert!(path("kept").exists());
        assert!(!path("removed").exists());
        assert!(!path("removed").with_extension("lock").exists());
        assert!(directory.path().join("unrelated.json").exists());
    }

    #[test]
    fn legacy_oauth_config_still_deserializes() {
        let config: McpConfig = serde_json::from_value(json!({
            "type": "http",
            "url": "https://mcp.example/mcp",
            "access_token": "legacy-secret",
            "expires_at": 123
        }))
        .unwrap();
        match config {
            McpConfig::Http {
                auth: Some(token), ..
            } => {
                assert_eq!(token.access_token, "legacy-secret");
                assert_eq!(token.expires_at, Some(123));
            }
            _ => panic!("expected legacy HTTP credentials"),
        }
    }
}
