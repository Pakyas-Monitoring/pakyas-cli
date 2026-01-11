use crate::cli::ApiKeyCommands;
use crate::client::ApiClient;
use crate::config::Context;
use crate::error::CliError;
use crate::output::{print_output, print_success, print_warning};
use anyhow::Result;
use chrono::{DateTime, Utc};
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tabled::Tabled;
use uuid::Uuid;

// ============================================================================
// API Types (matching backend responses)
// ============================================================================

/// Scope enum for API keys
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyScope {
    Read,
    Write,
    Manage,
}

impl std::fmt::Display for ApiKeyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read => write!(f, "read"),
            Self::Write => write!(f, "write"),
            Self::Manage => write!(f, "manage"),
        }
    }
}

impl std::str::FromStr for ApiKeyScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "manage" => Ok(Self::Manage),
            _ => Err(format!(
                "Invalid scope: '{}'. Valid scopes: read, write, manage",
                s
            )),
        }
    }
}

/// Response from GET /api/v1/api-keys
#[derive(Debug, Deserialize)]
struct ListApiKeysResponse {
    api_keys: Vec<ApiKeyResponse>,
    #[allow(dead_code)]
    total: usize,
}

/// Individual API key response (without the full key)
#[derive(Debug, Deserialize, Serialize)]
struct ApiKeyResponse {
    id: Uuid,
    name: String,
    key_prefix: String,
    scopes: Vec<ApiKeyScope>,
    #[allow(dead_code)]
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    is_expired: bool,
}

/// Response from POST /api/v1/api-keys (includes full key, shown once)
#[derive(Debug, Deserialize)]
struct ApiKeyCreated {
    #[allow(dead_code)]
    id: Uuid,
    name: String,
    key: String, // The full key - shown only once
    #[allow(dead_code)]
    key_prefix: String,
    #[allow(dead_code)]
    scopes: Vec<ApiKeyScope>,
    #[allow(dead_code)]
    expires_at: Option<DateTime<Utc>>,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

/// Request body for creating an API key
#[derive(Debug, Serialize)]
struct CreateApiKeyRequest {
    org_id: Uuid,
    name: String,
    scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_days: Option<i64>,
}

// ============================================================================
// Display Types
// ============================================================================

#[derive(Debug, Tabled, Serialize)]
struct ApiKeyRow {
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "PREFIX")]
    prefix: String,
    #[tabled(rename = "SCOPES")]
    scopes: String,
    #[tabled(rename = "CREATED")]
    created: String,
    #[tabled(rename = "LAST USED")]
    last_used: String,
    #[tabled(rename = "STATUS")]
    status: String,
}

// ============================================================================
// Commands
// ============================================================================

/// Handle api-key subcommands
pub async fn handle(ctx: &Context, command: ApiKeyCommands, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Some(org) = ctx.active_org_name() {
            eprintln!("[verbose] Active organization: {}", org);
        }
    }

    match command {
        ApiKeyCommands::List => list(ctx, verbose).await,
        ApiKeyCommands::Create {
            name,
            scopes,
            expires,
        } => create(ctx, name, scopes, expires, verbose).await,
        ApiKeyCommands::Revoke { id, yes } => revoke(ctx, &id, yes, verbose).await,
    }
}

/// List all API keys for the active organization
async fn list(ctx: &Context, verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/api-keys?org_id={}", org_id);

    if verbose {
        eprintln!("[verbose] Fetching API keys from: {}", url);
    }

    let spinner = create_spinner("Fetching API keys...");
    let response: ListApiKeysResponse = client.get(&url).await?;
    spinner.finish_and_clear();

    if verbose {
        eprintln!("[verbose] Found {} API key(s)", response.api_keys.len());
    }

    let rows: Vec<ApiKeyRow> = response
        .api_keys
        .into_iter()
        .map(|key| ApiKeyRow {
            name: key.name,
            id: key.id.to_string(),
            prefix: format!("pk_...{}", key.key_prefix),
            scopes: key
                .scopes
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            created: format_relative_time(Some(key.created_at)),
            last_used: format_relative_time(key.last_used_at),
            status: if key.is_expired {
                "expired".to_string()
            } else {
                "active".to_string()
            },
        })
        .collect();

    if rows.is_empty() {
        print_warning("No API keys found for this organization");
    } else {
        print_output(ctx, rows)?;
    }

    Ok(())
}

/// Create a new API key
async fn create(
    ctx: &Context,
    name: String,
    scopes: Vec<String>,
    expires: Option<i64>,
    _verbose: bool,
) -> Result<()> {
    let org_id = ctx.require_org()?;
    let org_uuid = Uuid::parse_str(org_id)
        .map_err(|_| CliError::Other("Invalid organization ID".to_string()))?;

    // Validate scopes
    let validated_scopes: Vec<String> = if scopes.is_empty() {
        vec!["read".to_string()]
    } else {
        scopes
            .iter()
            .map(|s| {
                s.parse::<ApiKeyScope>()
                    .map(|scope| scope.to_string())
                    .map_err(CliError::Other)
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    // Validate expires_in_days
    if let Some(days) = expires {
        if !(1..=365).contains(&days) {
            return Err(
                CliError::Other("Expiration must be between 1 and 365 days".to_string()).into(),
            );
        }
    }

    let client = ApiClient::new(ctx)?;
    let req = CreateApiKeyRequest {
        org_id: org_uuid,
        name: name.clone(),
        scopes: validated_scopes,
        expires_in_days: expires,
    };

    let spinner = create_spinner("Creating API key...");
    let created: ApiKeyCreated = client.post("/api/v1/api-keys", &req).await?;
    spinner.finish_and_clear();

    print_success(&format!("Created API key: {}", created.name));
    println!();
    println!("API Key: {}", created.key);
    println!();
    print_warning("Save this key now - it cannot be retrieved later!");

    Ok(())
}

/// Revoke an API key
async fn revoke(ctx: &Context, id: &str, skip_confirm: bool, _verbose: bool) -> Result<()> {
    let _org_id = ctx.require_org()?; // Ensure org context

    let key_uuid = Uuid::parse_str(id)
        .map_err(|_| CliError::Other("Invalid API key ID format".to_string()))?;

    if !skip_confirm {
        let confirm = Confirm::new()
            .with_prompt("Revoke this API key? This cannot be undone")
            .default(false)
            .interact()?;

        if !confirm {
            print_warning("Cancelled");
            return Ok(());
        }
    }

    let client = ApiClient::new(ctx)?;
    let spinner = create_spinner("Revoking API key...");
    let url = format!("/api/v1/api-keys/{}", key_uuid);
    client.delete(&url).await?;
    spinner.finish_and_clear();

    print_success("API key revoked successfully");

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Create a spinner for long-running operations
fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("Invalid spinner template"),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

/// Format a datetime as relative time (e.g., "5m ago", "2h ago")
fn format_relative_time(dt: Option<DateTime<Utc>>) -> String {
    let Some(dt) = dt else {
        return "never".to_string();
    };

    let now = Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_seconds() < 0 {
        // Future time
        let abs_diff = dt.signed_duration_since(now);
        if abs_diff.num_seconds() < 60 {
            "in <1m".to_string()
        } else if abs_diff.num_minutes() < 60 {
            format!("in {}m", abs_diff.num_minutes())
        } else if abs_diff.num_hours() < 24 {
            format!("in {}h", abs_diff.num_hours())
        } else {
            format!("in {}d", abs_diff.num_days())
        }
    } else if diff.num_seconds() < 60 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else {
        format!("{}d ago", diff.num_days())
    }
}
