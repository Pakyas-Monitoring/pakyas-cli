use crate::cli::OrgCommands;
use crate::client::ApiClient;
use crate::config::{Config, Context};
use crate::credentials::{CredentialsV2, OrgCredential};
use crate::error::CliError;
use crate::lock::GlobalLock;
use crate::output::{print_output, print_success, print_warning};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct OrganizationResponse {
    #[serde(flatten)]
    organization: Organization,
    user_role: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Organization {
    id: Uuid,
    name: String,
    #[serde(default)]
    timezone: Option<String>,
}

// ============================================================================
// Display Types
// ============================================================================

#[derive(Debug, Tabled, Serialize)]
struct OrgRow {
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "ROLE")]
    role: String,
    #[tabled(rename = "ACTIVE")]
    active: String,
}

// ============================================================================
// Commands
// ============================================================================

/// Handle organization subcommands.
pub async fn handle(ctx: &Context, command: OrgCommands, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
    }

    match command {
        OrgCommands::List => list(ctx, verbose).await,
        OrgCommands::Switch { name, no_prompt } => switch(ctx, &name, no_prompt, verbose).await,
    }
}

/// List all organizations the user belongs to.
async fn list(ctx: &Context, verbose: bool) -> Result<()> {
    let client = ApiClient::new(ctx)?;

    if verbose {
        eprintln!("[verbose] Fetching organizations from: /api/v1/organizations");
    }

    let orgs: Vec<OrganizationResponse> = client.get("/api/v1/organizations").await?;

    if verbose {
        eprintln!("[verbose] Found {} organization(s)", orgs.len());
    }

    let active_org_id = ctx.active_org_id();

    let rows: Vec<OrgRow> = orgs
        .into_iter()
        .map(|o| {
            let is_active = active_org_id == Some(o.organization.id.to_string().as_str());
            OrgRow {
                name: o.organization.name,
                id: o.organization.id.to_string(),
                role: o.user_role,
                active: if is_active { "*" } else { "" }.to_string(),
            }
        })
        .collect();

    print_output(ctx, rows)?;

    Ok(())
}

/// Switch the active organization.
///
/// This implements an atomic switch with these guarantees:
/// - If PAKYAS_API_KEY env var is set (and --ignore-env not passed), fail early
/// - Never write active_org_id to config until we have a valid key
/// - Use lock-release-relock pattern to avoid blocking during network calls
async fn switch(ctx: &Context, name_or_id: &str, no_prompt: bool, verbose: bool) -> Result<()> {
    // 1. Check env var block (unless --ignore-env was passed)
    let env_key_set = std::env::var("PAKYAS_API_KEY").is_ok();
    if env_key_set && !ctx.ignore_env() {
        return Err(CliError::EnvKeyBlocksSwitch.into());
    }

    // 2. Read initial state under lock
    let (initial_org_id, target_org_id, org_name, org_timezone, has_stored_key) = {
        let _lock = GlobalLock::acquire()?;
        let config = Config::load()?;
        let creds = CredentialsV2::load()?;

        // We need credentials to resolve org names via API
        // Use ApiClient which handles key selection
        let client = ApiClient::new(ctx)?;

        if verbose {
            eprintln!("[verbose] Searching for organization: {}", name_or_id);
        }

        let orgs: Vec<OrganizationResponse> = client.get("/api/v1/organizations").await?;

        // Find org by name or ID
        let org = orgs
            .iter()
            .find(|o| {
                o.organization.name.eq_ignore_ascii_case(name_or_id)
                    || o.organization.id.to_string() == name_or_id
            })
            .ok_or_else(|| CliError::OrgNotFound(name_or_id.to_string()))?;

        let target_id = org.organization.id.to_string();
        let has_key = creds.has_key_for_org(&target_id);

        if verbose {
            eprintln!(
                "[verbose] Found organization: {} ({}), has_stored_key: {}",
                org.organization.name, target_id, has_key
            );
        }

        (
            config.active_org_id.clone(),
            target_id,
            org.organization.name.clone(),
            org.organization.timezone.clone(),
            has_key,
        )
    };
    // Lock released here

    // 3. Handle missing key (network calls without lock)
    if !has_stored_key {
        if no_prompt {
            // In CI/script mode, fail with clear instructions
            return Err(CliError::NoKeyForOrg(target_org_id).into());
        }

        // TODO: Interactive flow will be implemented in a follow-up
        // For now, if no key stored, fail with instruction
        eprintln!(
            "No API key stored for organization '{}' ({}).",
            org_name, target_org_id
        );
        eprintln!("Run: pakyas login");
        return Err(CliError::NoKeyForOrg(target_org_id).into());
    }

    // 4. Write updated state under lock (re-acquire)
    {
        let _lock = GlobalLock::acquire()?;

        // Re-read to detect concurrent changes
        let mut config = Config::load()?;
        let mut creds = CredentialsV2::load()?;

        // Check for concurrent modification
        if config.active_org_id != initial_org_id {
            return Err(CliError::ConcurrentModification(
                "Active org changed in another terminal. Please retry.".to_string(),
            )
            .into());
        }

        // Update last_verified timestamp
        if let Some(org_cred) = creds.get_for_org(&target_org_id) {
            creds.set_for_org(
                &target_org_id,
                OrgCredential {
                    api_key: org_cred.api_key.clone(),
                    key_id: org_cred.key_id.clone(),
                    label: org_cred.label.clone(),
                    added_at: org_cred.added_at,
                    last_verified: Some(Utc::now()),
                },
            );
            creds.save()?;
        }

        // Update config
        config.active_org_id = Some(target_org_id.clone());
        config.active_org_name = Some(org_name.clone());
        config.active_org_timezone = org_timezone;
        // Clear project when switching orgs (projects are org-specific)
        config.active_project_id = None;
        config.active_project_name = None;
        config.save()?;
    }
    // Lock released here

    print_success(&format!("Switched to organization: {}", org_name));

    // Warn if env var is still set (user used --ignore-env)
    if env_key_set && ctx.ignore_env() {
        print_warning(
            "Your shell sets PAKYAS_API_KEY. Other commands will use it unless you add --ignore-env or unset the variable.",
        );
    }

    Ok(())
}
