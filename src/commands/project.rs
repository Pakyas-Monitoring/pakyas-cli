use crate::cli::ProjectCommands;
use crate::client::ApiClient;
use crate::config::{Config, Context};
use crate::error::CliError;
use crate::output::{print_output, print_success};
use anyhow::Result;
use dialoguer::Input;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
struct Project {
    id: Uuid,
    org_id: Uuid,
    name: String,
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateProjectRequest {
    org_id: Uuid,
    name: String,
    description: Option<String>,
}

// ============================================================================
// Display Types
// ============================================================================

#[derive(Debug, Tabled, Serialize)]
struct ProjectRow {
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "DESCRIPTION")]
    description: String,
    #[tabled(rename = "ACTIVE")]
    active: String,
}

// ============================================================================
// Commands
// ============================================================================

/// Handle project subcommands.
pub async fn handle(ctx: &Context, command: ProjectCommands, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Some(org) = ctx.active_org_name() {
            eprintln!("[verbose] Active organization: {}", org);
        }
    }

    match command {
        ProjectCommands::List => list(ctx, verbose).await,
        ProjectCommands::Create { name, description } => {
            create(ctx, name, description, verbose).await
        }
        ProjectCommands::Switch { name } => switch(ctx, name, verbose).await,
    }
}

/// List all projects in the active organization.
async fn list(ctx: &Context, verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/projects?org_id={}", org_id);

    if verbose {
        eprintln!("[verbose] Fetching projects from: {}", url);
    }

    let projects: Vec<Project> = client.get(&url).await?;

    if verbose {
        eprintln!("[verbose] Found {} project(s)", projects.len());
    }

    let active_project_id = ctx.active_project_id();

    let rows: Vec<ProjectRow> = projects
        .into_iter()
        .map(|p| {
            let is_active = active_project_id == Some(p.id.to_string().as_str());
            ProjectRow {
                name: p.name,
                id: p.id.to_string(),
                description: p.description.unwrap_or_default(),
                active: if is_active { "*" } else { "" }.to_string(),
            }
        })
        .collect();

    print_output(ctx, rows)?;

    Ok(())
}

/// Create a new project.
async fn create(
    ctx: &Context,
    name: Option<String>,
    description: Option<String>,
    verbose: bool,
) -> Result<()> {
    let org_id = ctx.require_org()?;

    // Get name interactively if not provided
    let name = match name {
        Some(n) => n,
        None => Input::new().with_prompt("Project name").interact_text()?,
    };

    // Get description interactively if not provided (optional)
    let description = match description {
        Some(d) => Some(d),
        None => {
            let desc: String = Input::new()
                .with_prompt("Description (optional)")
                .allow_empty(true)
                .interact_text()?;
            if desc.is_empty() { None } else { Some(desc) }
        }
    };

    let org_uuid = Uuid::parse_str(org_id)
        .map_err(|_| CliError::Other("Invalid organization ID".to_string()))?;

    let client = ApiClient::new(ctx)?;
    let req = CreateProjectRequest {
        org_id: org_uuid,
        name: name.clone(),
        description,
    };

    if verbose {
        eprintln!("[verbose] Creating project: {}", name);
    }

    let project: Project = client.post("/api/v1/projects", &req).await?;

    if verbose {
        eprintln!("[verbose] Created project with ID: {}", project.id);
    }

    // Set as active project
    let mut config = Config::load()?;
    config.active_project_id = Some(project.id.to_string());
    config.active_project_name = Some(project.name.clone());
    config.save()?;

    print_success(&format!("Created project: {}", project.name));
    print_success("Set as active project");

    Ok(())
}

/// Switch the active project.
async fn switch(ctx: &Context, name_parts: Vec<String>, verbose: bool) -> Result<()> {
    // Check if user forgot quotes for multi-word project name
    if name_parts.len() > 1 {
        return Err(CliError::Other(format!(
            "Project name appears to have spaces. Did you forget quotes?\n\
             Try: pakyas project switch \"{}\"",
            name_parts.join(" ")
        ))
        .into());
    }
    let name_or_id = &name_parts[0];

    if verbose {
        eprintln!("[verbose] Searching for project: {}", name_or_id);
    }

    let org_id = ctx.require_org()?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/projects?org_id={}", org_id);
    let projects: Vec<Project> = client.get(&url).await?;

    // Find project by name or ID
    let project = projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name_or_id) || p.id.to_string() == *name_or_id)
        .ok_or_else(|| CliError::ProjectNotFound(name_or_id.to_string()))?;

    if verbose {
        eprintln!("[verbose] Found project: {} ({})", project.name, project.id);
    }

    // Update config
    let mut config = Config::load()?;
    config.active_project_id = Some(project.id.to_string());
    config.active_project_name = Some(project.name.clone());
    config.save()?;

    print_success(&format!("Switched to project: {}", project.name));

    Ok(())
}
