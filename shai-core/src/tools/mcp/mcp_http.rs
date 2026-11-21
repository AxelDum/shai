use async_trait::async_trait;
use rmcp::{
    model::ClientConfig,
    service::{ClientInitializeError, RunningService, ServiceExt},
    transport::{
        auth::{
            AuthClient, AuthError, AuthorizationManager, CredentialStore, OAuthTokenResponse,
            StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
        StreamableHttpClientTransport,
    },
    RoleClient,
};

use super::{
    mcp::{call_tool_params, content_to_string, McpClient, McpToolDescription},
    mcp_config::{McpCredentialStore, OAuthToken},
    mcp_oauth::{authorize, default_browser_opener, BrowserOpener},
};
use crate::tools::{ToolCall, ToolResult};

/// Marks a bearer token migrated from the agent config into the credential store.
const LEGACY_TOKEN_CLIENT_ID: &str = "shai-legacy-token";

pub struct HttpClient {
    url: String,
    legacy_token: Option<OAuthToken>,
    credential_store: McpCredentialStore,
    open_browser: BrowserOpener,
    service: Option<RunningService<RoleClient, ClientConfig>>,
}

impl HttpClient {
    pub fn new(url: String) -> Self {
        Self::new_with_auth(url, None)
    }

    pub fn new_with_auth(url: String, bearer_token: Option<String>) -> Self {
        let legacy_token = bearer_token.map(|access_token| OAuthToken {
            access_token,
            expires_at: None,
        });
        Self::with_credential_store(url, legacy_token, McpCredentialStore::in_memory())
    }

    pub(crate) fn with_credential_store(
        url: String,
        legacy_token: Option<OAuthToken>,
        credential_store: McpCredentialStore,
    ) -> Self {
        Self {
            url,
            legacy_token,
            credential_store,
            open_browser: default_browser_opener(),
            service: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_browser_opener<F>(mut self, opener: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.open_browser = Arc::new(opener);
        self
    }

    async fn manager(&self) -> Result<AuthorizationManager, rmcp::transport::auth::AuthError> {
        let mut manager = AuthorizationManager::new(self.url.clone()).await?;
        manager.set_credential_store(self.credential_store.clone());
        Ok(manager)
    }

    async fn connect_authorized(
        &mut self,
        manager: AuthorizationManager,
    ) -> Result<(), Box<ClientInitializeError>> {
        let client = AuthClient::new(reqwest::Client::new(), manager);
        let transport = StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(self.url.as_str()),
        );
        let service = ClientConfig::default()
            .serve(transport)
            .await
            .map_err(Box::new)?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.service = Some(service);
        Ok(())
    }

    async fn connect_legacy(&mut self) -> Result<(), Box<ClientInitializeError>> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.as_str());
        if let Some(token) = self
            .legacy_token
            .as_ref()
            .filter(|token| !token.is_expired())
        {
            config = config.auth_header(token.access_token.as_str());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let service = ClientConfig::default()
            .serve(transport)
            .await
            .map_err(Box::new)?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.service = Some(service);
        Ok(())
    }

    /// Move a still-valid embedded token into the credential store so it can be
    /// removed from the agent config.
    async fn migrate_legacy_token(&self) -> Result<(), AuthError> {
        let Some(token) = self
            .legacy_token
            .as_ref()
            .filter(|token| !token.is_expired())
        else {
            return Ok(());
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut response = serde_json::json!({
            "access_token": token.access_token,
            "token_type": "Bearer",
        });
        if let Some(expires_at) = token.expires_at {
            response["expires_in"] = (expires_at - now as i64).max(0).into();
        }
        let response: OAuthTokenResponse = serde_json::from_value(response)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        self.credential_store
            .save(StoredCredentials::new(
                LEGACY_TOKEN_CLIENT_ID.to_string(),
                Some(response),
                Vec::new(),
                Some(now),
            ))
            .await
    }

    /// A migrated token keeps the legacy behaviour: it is sent as is, without
    /// OAuth discovery, until the server rejects it.
    async fn load_migrated_token(&self) -> Result<Option<OAuthToken>, AuthError> {
        let Some(stored) = self.credential_store.load().await? else {
            return Ok(None);
        };
        if stored.client_id != LEGACY_TOKEN_CLIENT_ID {
            return Ok(None);
        }
        let response = serde_json::to_value(&stored.token_response)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        Ok(response["access_token"]
            .as_str()
            .map(|access_token| OAuthToken {
                access_token: access_token.to_string(),
                expires_at: response["expires_in"]
                    .as_i64()
                    .zip(stored.token_received_at)
                    .map(|(expires_in, received_at)| received_at as i64 + expires_in),
            }))
    }
}

#[async_trait]
impl McpClient for HttpClient {
    async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.service.is_some() {
            return Ok(());
        }

        let migrated_token = self.load_migrated_token().await?;
        let migrated = migrated_token.is_some();
        let mut manager = self.manager().await?;
        let challenge = if !migrated && manager.initialize_from_store().await? {
            match self.connect_authorized(manager).await {
                Ok(()) => return Ok(()),
                Err(error) if error.is_authorization_required() => {
                    error.auth_challenge().map(str::to_owned)
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            if self.legacy_token.is_none() {
                self.legacy_token = migrated_token;
            }
            match self.connect_legacy().await {
                Ok(()) => {
                    if !migrated {
                        if let Err(error) = self.migrate_legacy_token().await {
                            tracing::warn!(%error, "failed to migrate legacy MCP token");
                        }
                    }
                    return Ok(());
                }
                Err(error) if error.is_authorization_required() => {
                    error.auth_challenge().map(str::to_owned)
                }
                Err(error) => return Err(error.into()),
            }
        };

        let manager = self.manager().await?;
        let manager = authorize(manager, challenge, self.open_browser.clone()).await?;
        self.connect_authorized(manager).await?;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(service) = self.service.take() {
            service.cancel().await?;
        }
        Ok(())
    }

    async fn list_tools(
        &self,
    ) -> Result<Vec<McpToolDescription>, Box<dyn std::error::Error + Send + Sync>> {
        let service = self.service.as_ref().ok_or("Not connected")?;
        let tools_result = service.list_tools(None).await?;

        let tool_descriptions = tools_result
            .tools
            .into_iter()
            .map(|tool| McpToolDescription {
                name: tool.name.to_string(),
                description: tool.description.unwrap_or_default().to_string(),
                parameters_schema: serde_json::Value::Object((*tool.input_schema).clone()),
            })
            .collect();

        Ok(tool_descriptions)
    }

    async fn execute_tool(
        &self,
        tool_call: ToolCall,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let service = self.service.as_ref().ok_or("Not connected")?;

        let result = service.call_tool(call_tool_params(&tool_call)).await?;

        Ok(ToolResult::success(content_to_string(result.content)))
    }
}

#[cfg(test)]
use std::sync::Arc;
