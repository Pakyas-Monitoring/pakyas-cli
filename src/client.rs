use crate::config::Context;
use crate::credentials::CredentialsV2;
use crate::error::CliError;
use anyhow::Result;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

const TIMEOUT_SECS: u64 = 30;

/// Source of the API key used for authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// From PAKYAS_API_KEY environment variable
    Env,
    /// From credentials.orgs[org_id]
    OrgStored,
    /// From credentials.legacy_api_key (V1 migration)
    Legacy,
}

pub struct ApiClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    /// The org ID to send in X-Pakyas-Org header (if known)
    org_id: Option<String>,
    /// Where the API key came from (for debugging)
    #[allow(dead_code)]
    auth_source: Option<AuthSource>,
    /// Enable verbose output for debugging
    verbose: bool,
}

impl ApiClient {
    /// Create a new API client with automatic key selection.
    ///
    /// Key selection precedence:
    /// 1. PAKYAS_API_KEY env var (unless ignore_env is set)
    /// 2. Stored key for active org (from CredentialsV2)
    /// 3. Legacy key (from V1 migration)
    /// 4. Error: no key available
    pub fn new(ctx: &Context) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()?;

        let base_url = ctx.api_url();
        let active_org_id = ctx.active_org_id().map(|s| s.to_string());

        // Select API key based on precedence
        let (api_key, auth_source) = Self::select_key(ctx, active_org_id.as_deref())?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Some(api_key),
            org_id: active_org_id,
            auth_source: Some(auth_source),
            verbose: false,
        })
    }

    /// Select the API key based on precedence rules.
    ///
    /// Precedence:
    /// 1. PAKYAS_API_KEY env var (unless ignore_env is set)
    /// 2. Stored key for active org
    /// 3. Legacy key
    /// 4. Error
    fn select_key(
        ctx: &Context,
        active_org_id: Option<&str>,
    ) -> Result<(String, AuthSource), CliError> {
        // 1. Check env var (unless ignore_env)
        if !ctx.ignore_env() {
            if let Ok(key) = std::env::var("PAKYAS_API_KEY") {
                return Ok((key, AuthSource::Env));
            }
        }

        // Load credentials from file (env var already checked above)
        let creds = Self::load_credentials_from_file()?;

        // 2. Check for org-specific key
        if let Some(org_id) = active_org_id {
            if let Some(org_cred) = creds.get_for_org(org_id) {
                return Ok((org_cred.api_key.clone(), AuthSource::OrgStored));
            }
        }

        // 3. Check for legacy key
        if let Some(legacy_key) = creds.legacy_key() {
            return Ok((legacy_key.to_string(), AuthSource::Legacy));
        }

        // 4. No key available
        Err(CliError::NotAuthenticated)
    }

    /// Load CredentialsV2 from file only (ignoring env var).
    /// This is used internally to check file-based credentials
    /// after we've already handled the env var check.
    fn load_credentials_from_file() -> Result<CredentialsV2, CliError> {
        let path = CredentialsV2::path()?;
        if !path.exists() {
            return Ok(CredentialsV2::default());
        }

        let content = std::fs::read_to_string(&path).map_err(CliError::ConfigRead)?;

        // Try V2 first
        if let Ok(v2) = serde_json::from_str::<CredentialsV2>(&content) {
            if v2.version == 2 {
                return Ok(v2);
            }
        }

        // Try V1 migration - use a simple inline deserialize
        #[derive(serde::Deserialize)]
        struct LegacyV1 {
            api_key: Option<String>,
        }

        serde_json::from_str::<LegacyV1>(&content)
            .map(|v1| CredentialsV2 {
                version: 2,
                orgs: std::collections::HashMap::new(),
                legacy_api_key: v1.api_key,
            })
            .map_err(|_| CliError::CredentialsCorrupted)
    }

    pub fn with_api_key(ctx: &Context, api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()?;

        let base_url = ctx.api_url();
        let org_id = ctx.active_org_id().map(|s| s.to_string());

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Some(api_key),
            org_id,
            auth_source: None,
            verbose: false,
        })
    }

    /// Create a client with explicit base URL and API key (for testing)
    pub fn with_base_url(base_url: String, api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            org_id: None,
            auth_source: None,
            verbose: false,
        })
    }

    /// Enable verbose output for debugging
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    fn require_auth(&self) -> Result<&str, CliError> {
        self.api_key.as_deref().ok_or(CliError::NotAuthenticated)
    }

    /// Add common headers including X-Pakyas-Org if org_id is known.
    fn add_common_headers(
        &self,
        builder: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        let builder = builder.header("Authorization", format!("Bearer {}", api_key));

        // Add org header for server-side validation (future-proofing)
        if let Some(ref org_id) = self.org_id {
            builder.header("X-Pakyas-Org", org_id)
        } else {
            builder
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.get(&url);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        self.handle_response(response).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.post(&url).json(body);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        self.handle_response(response).await
    }

    pub async fn post_no_body<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.post(&url);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        self.handle_response(response).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.put(&url).json(body);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        self.handle_response(response).await
    }

    /// PUT request that expects no response body (204 No Content)
    pub async fn put_no_response<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.put(&url).json(body);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::handle_error(response).await.into())
        }
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let request = self.client.delete(&url);
        let request = self.add_common_headers(request, api_key);

        let response = request.send().await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::handle_error(response).await.into())
        }
    }

    /// Make an unauthenticated request (for login)
    pub async fn post_unauth<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let response = self.client.post(&url).json(body).send().await?;

        self.handle_response(response).await
    }

    async fn handle_response<T: DeserializeOwned>(&self, response: Response) -> Result<T> {
        let status = response.status();
        if status.is_success() {
            let text = response.text().await?;
            if self.verbose {
                eprintln!("[verbose] Response body: {}", text);
            }
            serde_json::from_str(&text).map_err(|e| {
                if self.verbose {
                    eprintln!("[verbose] Deserialization error: {}", e);
                }
                anyhow::anyhow!("error decoding response body: {}", e)
            })
        } else {
            Err(Self::handle_error(response).await.into())
        }
    }

    async fn handle_error(response: Response) -> CliError {
        let status = response.status();

        #[derive(serde::Deserialize)]
        struct ErrorResponse {
            error: Option<String>,
            message: Option<String>,
            key_org_name: Option<String>,
            requested_org_id: Option<String>,
        }

        let error_body = response.json::<ErrorResponse>().await.ok();

        // Check for ORG_KEY_MISMATCH structured error
        if status == StatusCode::FORBIDDEN {
            if let Some(ref body) = error_body {
                if body.error.as_deref() == Some("ORG_KEY_MISMATCH") {
                    if let (Some(key_org), Some(requested)) =
                        (&body.key_org_name, &body.requested_org_id)
                    {
                        return CliError::OrgKeyMismatch {
                            key_org: key_org.clone(),
                            active_org: requested.clone(),
                        };
                    }
                }
            }
        }

        let error_msg = error_body.and_then(|e| e.error.or(e.message));

        match status {
            StatusCode::UNAUTHORIZED => CliError::NotAuthenticated,
            StatusCode::FORBIDDEN => {
                CliError::api(error_msg.unwrap_or_else(|| "Permission denied".to_string()))
            }
            StatusCode::NOT_FOUND => {
                CliError::api(error_msg.unwrap_or_else(|| "Resource not found".to_string()))
            }
            _ => CliError::api(
                error_msg.unwrap_or_else(|| format!("Request failed with status {}", status)),
            ),
        }
    }
}

/// Check if a string looks like an org ID (org_<alphanumeric>).
/// Used to distinguish org IDs from org names/aliases.
pub fn is_org_id(s: &str) -> bool {
    s.starts_with("org_") && s.len() > 4 && s[4..].chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_org_id_valid() {
        assert!(is_org_id("org_abc123"));
        assert!(is_org_id("org_ABC123"));
        assert!(is_org_id("org_test_org_123"));
        assert!(is_org_id("org_12345"));
    }

    #[test]
    fn test_is_org_id_invalid() {
        assert!(!is_org_id("org_")); // Too short
        assert!(!is_org_id("org")); // No underscore prefix part
        assert!(!is_org_id("organization_abc")); // Wrong prefix
        assert!(!is_org_id("Acme Corp")); // Org name, not ID
        assert!(!is_org_id("")); // Empty
        assert!(!is_org_id("org_abc-def")); // Hyphen not allowed
    }
}
