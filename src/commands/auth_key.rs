//! Auth key management commands.
//!
//! Provides commands for managing stored API keys:
//! - list: Show all stored keys by organization
//! - set: Import/paste a key for an organization
//! - verify: Validate a stored key is still valid
//! - rm: Remove a stored key

use crate::cli::AuthKeyCommands;
use crate::client::ApiClient;
use crate::config::Context;
use crate::credentials::{CredentialsV2, OrgCredential, validate_api_key};
use crate::error::CliError;
use crate::lock::GlobalLock;
use crate::output::{print_error, print_info, print_success, print_warning};
use anyhow::Result;
use chrono::Utc;
use dialoguer::{Confirm, Password};
use serde::Deserialize;

/// Handle auth key subcommands.
pub async fn handle(ctx: &Context, command: AuthKeyCommands, verbose: bool) -> Result<()> {
    match command {
        AuthKeyCommands::List => list(verbose).await,
        AuthKeyCommands::Set { org, key } => set(ctx, &org, key.as_deref(), verbose).await,
        AuthKeyCommands::Verify { org } => verify(ctx, org.as_deref(), verbose).await,
        AuthKeyCommands::Rm { org, legacy, yes } => rm(org.as_deref(), legacy, yes, verbose).await,
    }
}

/// List all stored API keys by organization.
async fn list(verbose: bool) -> Result<()> {
    if verbose {
        if let Ok(path) = CredentialsV2::path() {
            eprintln!("[verbose] Credentials path: {}", path.display());
        }
    }

    // Load credentials without env var override to see actual stored keys
    let creds = load_credentials_ignoring_env()?;

    if creds.orgs.is_empty() && creds.legacy_api_key.is_none() {
        print_info("No stored API keys found.");
        print_info("Run 'pakyas login' or 'pakyas auth key set --org <ORG_ID>' to add one.");
        return Ok(());
    }

    println!("Stored API Keys:");
    println!();

    // Show per-org keys
    if !creds.orgs.is_empty() {
        for (org_id, cred) in &creds.orgs {
            let key_preview = format_key_preview(&cred.api_key);
            let label = cred.label.as_deref().unwrap_or("(no label)");
            let added = cred.added_at.format("%Y-%m-%d %H:%M").to_string();
            let verified = cred
                .last_verified
                .map(|v| v.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "(never)".to_string());

            println!("  Organization: {}", org_id);
            println!("    Key:           {}", key_preview);
            println!("    Label:         {}", label);
            println!("    Added:         {}", added);
            println!("    Last verified: {}", verified);
            println!();
        }
    }

    // Show legacy key if present
    if let Some(ref legacy_key) = creds.legacy_api_key {
        let key_preview = format_key_preview(legacy_key);
        println!("  Legacy Key (not associated with any org):");
        println!("    Key: {}", key_preview);
        println!();
        print_warning(
            "Run 'pakyas org switch <org>' to migrate the legacy key, or 'pakyas auth key rm --legacy' to remove it.",
        );
    }

    Ok(())
}

/// Set/import an API key for an organization.
async fn set(ctx: &Context, org_id: &str, key: Option<&str>, verbose: bool) -> Result<()> {
    // Get the API key (from argument or prompt)
    let api_key = match key {
        Some(k) => k.to_string(),
        None => {
            let key: String = Password::new().with_prompt("Paste API key").interact()?;
            key
        }
    };

    // Validate format
    validate_api_key(&api_key)?;

    // Validate the key works and belongs to the right org
    if verbose {
        eprintln!("[verbose] Validating key against /api/v1/me");
    }

    let me_response = validate_key_with_server(ctx, &api_key).await?;

    // Check org matches
    if me_response.org_id != org_id {
        print_error(&format!(
            "Key belongs to org '{}' ({}), not '{}'",
            me_response.org_name.as_deref().unwrap_or("unknown"),
            me_response.org_id,
            org_id
        ));
        return Err(CliError::OrgKeyMismatch {
            key_org: me_response.org_id,
            active_org: org_id.to_string(),
        }
        .into());
    }

    // Store the key
    let _lock = GlobalLock::acquire()?;
    let mut creds = load_credentials_ignoring_env()?;

    let device_label = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|h| format!("{}-{}", h, Utc::now().format("%Y-%m-%d")))
        .unwrap_or_else(|| format!("CLI-{}", Utc::now().format("%Y-%m-%d")));

    creds.set_for_org(
        org_id,
        OrgCredential {
            api_key,
            key_id: None,
            label: Some(device_label),
            added_at: Utc::now(),
            last_verified: Some(Utc::now()),
        },
    );
    creds.save()?;

    print_success(&format!(
        "API key stored for organization: {} ({})",
        me_response.org_name.as_deref().unwrap_or(org_id),
        org_id
    ));

    Ok(())
}

/// Verify a stored API key is valid.
async fn verify(ctx: &Context, org_id: Option<&str>, verbose: bool) -> Result<()> {
    // Determine which org to verify
    let target_org = match org_id {
        Some(o) => o.to_string(),
        None => ctx
            .active_org_id()
            .ok_or_else(|| CliError::NoOrgSelected)?
            .to_string(),
    };

    // Load credentials without env var
    let creds = load_credentials_ignoring_env()?;

    // Get the key for this org
    let org_cred = creds.get_for_org(&target_org).ok_or_else(|| {
        CliError::api(format!(
            "No stored key for org '{}'. Run 'pakyas auth key set --org {}'",
            target_org, target_org
        ))
    })?;

    if verbose {
        eprintln!("[verbose] Verifying key for org: {}", target_org);
        eprintln!(
            "[verbose] Key preview: {}",
            format_key_preview(&org_cred.api_key)
        );
    }

    // Validate with server
    match validate_key_with_server(ctx, &org_cred.api_key).await {
        Ok(me_response) => {
            // Update last_verified timestamp
            {
                let _lock = GlobalLock::acquire()?;
                let mut creds = load_credentials_ignoring_env()?;
                if let Some(cred) = creds.get_for_org_mut(&target_org) {
                    cred.last_verified = Some(Utc::now());
                }
                creds.save()?;
            }

            print_success("Key is valid!");
            println!();
            println!(
                "  Organization: {} ({})",
                me_response.org_name.as_deref().unwrap_or("unknown"),
                me_response.org_id
            );
            if let Some(email) = me_response.email {
                println!("  User:         {}", email);
            }
            println!(
                "  Last verified: {}",
                Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            );
        }
        Err(e) => {
            print_error(&format!("Key validation failed: {}", e));
            print_info(&format!(
                "Run 'pakyas auth key set --org {}' to update the key.",
                target_org
            ));
            return Err(e);
        }
    }

    Ok(())
}

/// Remove a stored API key.
async fn rm(org_id: Option<&str>, legacy: bool, yes: bool, verbose: bool) -> Result<()> {
    if !legacy && org_id.is_none() {
        return Err(CliError::api("Must specify --org <ORG_ID> or --legacy").into());
    }

    let _lock = GlobalLock::acquire()?;
    let mut creds = load_credentials_ignoring_env()?;

    if legacy {
        // Remove legacy key
        if creds.legacy_api_key.is_none() {
            print_info("No legacy key to remove.");
            return Ok(());
        }

        if !yes {
            let confirm = Confirm::new()
                .with_prompt("Remove legacy API key?")
                .default(false)
                .interact()?;

            if !confirm {
                print_info("Cancelled.");
                return Ok(());
            }
        }

        creds.remove_legacy_key();
        creds.save()?;
        print_success("Legacy API key removed.");
    } else if let Some(org) = org_id {
        // Remove key for specific org
        if !creds.has_key_for_org(org) {
            print_info(&format!("No stored key for organization '{}'.", org));
            return Ok(());
        }

        if verbose {
            if let Some(cred) = creds.get_for_org(org) {
                eprintln!(
                    "[verbose] Removing key: {}",
                    format_key_preview(&cred.api_key)
                );
            }
        }

        if !yes {
            let confirm = Confirm::new()
                .with_prompt(format!("Remove API key for organization '{}'?", org))
                .default(false)
                .interact()?;

            if !confirm {
                print_info("Cancelled.");
                return Ok(());
            }
        }

        creds.remove_for_org(org);
        creds.save()?;
        print_success(&format!("API key removed for organization: {}", org));
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Format a key preview (first 12 chars + ...)
fn format_key_preview(key: &str) -> String {
    if key.len() > 12 {
        format!("{}...", &key[..12])
    } else {
        key.to_string()
    }
}

/// Load credentials from file, ignoring PAKYAS_API_KEY env var.
///
/// This is used for key management commands where we want to see/modify
/// the actual stored credentials, not the env var override.
fn load_credentials_ignoring_env() -> Result<CredentialsV2, CliError> {
    let path = CredentialsV2::path()?;

    if !path.exists() {
        return Ok(CredentialsV2::default());
    }

    let content = std::fs::read_to_string(&path).map_err(CliError::ConfigRead)?;

    // Try to parse as V2 first
    if let Ok(v2) = serde_json::from_str::<CredentialsV2>(&content) {
        if v2.version == 2 {
            return Ok(v2);
        }
    }

    // Try to parse as V1 and migrate
    #[derive(Deserialize)]
    struct CredentialsV1 {
        api_key: Option<String>,
    }

    match serde_json::from_str::<CredentialsV1>(&content) {
        Ok(v1) => Ok(CredentialsV2 {
            version: 2,
            orgs: std::collections::HashMap::new(),
            legacy_api_key: v1.api_key,
        }),
        Err(_) => Err(CliError::CredentialsCorrupted),
    }
}

/// Response from /api/v1/me endpoint
#[derive(Debug, Deserialize)]
struct MeResponse {
    #[serde(default)]
    org_id: String,
    #[serde(default)]
    org_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// Validate an API key against the server's /api/v1/me endpoint.
async fn validate_key_with_server(ctx: &Context, api_key: &str) -> Result<MeResponse> {
    let client = ApiClient::with_api_key(ctx, api_key.to_string())?;
    let me: MeResponse = client.get("/api/v1/me").await?;
    Ok(me)
}
