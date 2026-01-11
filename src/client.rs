use crate::config::Context;
use crate::credentials::Credentials;
use crate::error::CliError;
use anyhow::Result;
use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

const TIMEOUT_SECS: u64 = 30;

pub struct ApiClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl ApiClient {
    pub fn new(ctx: &Context) -> Result<Self> {
        let creds = Credentials::load()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()?;

        let base_url = ctx.api_url();
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: creds.api_key,
        })
    }

    pub fn with_api_key(ctx: &Context, api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()?;

        let base_url = ctx.api_url();
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Some(api_key),
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
        })
    }

    fn require_auth(&self) -> Result<&str, CliError> {
        self.api_key.as_deref().ok_or(CliError::NotAuthenticated)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

        Self::handle_response(response).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(body)
            .send()
            .await?;

        Self::handle_response(response).await
    }

    pub async fn post_no_body<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

        Self::handle_response(response).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(body)
            .send()
            .await?;

        Self::handle_response(response).await
    }

    /// PUT request that expects no response body (204 No Content)
    pub async fn put_no_response<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(body)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::handle_error(response).await.into())
        }
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let api_key = self.require_auth()?;
        let url = format!("{}{}", self.base_url, path);

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

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

        Self::handle_response(response).await
    }

    async fn handle_response<T: DeserializeOwned>(response: Response) -> Result<T> {
        let status = response.status();
        if status.is_success() {
            let data = response.json::<T>().await?;
            Ok(data)
        } else {
            Err(Self::handle_error(response).await.into())
        }
    }

    async fn handle_error(response: Response) -> CliError {
        let status = response.status();

        // Try to parse error message from response body
        #[derive(serde::Deserialize)]
        struct ErrorResponse {
            error: Option<String>,
            message: Option<String>,
        }

        let error_msg = match response.json::<ErrorResponse>().await {
            Ok(err) => err.error.or(err.message),
            Err(_) => None,
        };

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
