use crate::cli::OrgCommands;
use crate::client::ApiClient;
use crate::config::{Config, Context};
use crate::error::CliError;
use crate::output::{print_output, print_success};
use anyhow::Result;
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
        OrgCommands::Switch { name, no_prompt: _ } => switch(ctx, &name, verbose).await,
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
async fn switch(ctx: &Context, name_or_id: &str, verbose: bool) -> Result<()> {
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

    if verbose {
        eprintln!(
            "[verbose] Found organization: {} ({})",
            org.organization.name, org.organization.id
        );
    }

    // Update config
    let mut config = Config::load()?;
    config.active_org_id = Some(org.organization.id.to_string());
    config.active_org_name = Some(org.organization.name.clone());
    // Cache org timezone for dry-run/JSON output
    // IMPORTANT: Always update, even if org.timezone is None
    // This prevents leaking the previous org's timezone
    config
        .active_org_timezone
        .clone_from(&org.organization.timezone);
    // Clear project when switching orgs (projects are org-specific)
    config.active_project_id = None;
    config.active_project_name = None;
    config.save()?;

    print_success(&format!(
        "Switched to organization: {}",
        org.organization.name
    ));

    Ok(())
}
