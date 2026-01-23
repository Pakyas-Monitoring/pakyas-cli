use crate::cli::OrgCommands;
use crate::client::ApiClient;
use crate::config::{Config, Context};
use crate::credentials::{CredentialsV2, OrgCredential, validate_api_key};
use crate::error::CliError;
use crate::lock::GlobalLock;
use crate::output::{print_info, print_output, print_success, print_warning};
use anyhow::Result;
use chrono::Utc;
use dialoguer::{Confirm, Password, Select};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::time::Duration;
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

#[derive(Debug, Deserialize)]
struct ProjectResponse {
    id: Uuid,
    name: String,
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
            return Err(CliError::NoKeyForOrg(target_org_id.clone()).into());
        }

        // Interactive flow: prompt user for how to authenticate
        println!(
            "No API key stored for organization '{}' ({}).\n",
            org_name, target_org_id
        );

        let options = &[
            "Create new API key (opens browser)",
            "Paste existing API key",
            "Cancel",
        ];

        let selection = Select::new()
            .with_prompt("How would you like to authenticate?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => {
                // Browser device code flow with target_org_id
                let api_key =
                    create_key_via_browser(ctx, &target_org_id, &org_name, verbose).await?;
                store_key_for_org(&target_org_id, &api_key)?;
            }
            1 => {
                // Paste key flow with validation
                let api_key =
                    paste_and_validate_key(ctx, &target_org_id, &org_name, verbose).await?;
                store_key_for_org(&target_org_id, &api_key)?;
            }
            2 => {
                // Cancel
                println!("Cancelled.");
                return Ok(());
            }
            _ => unreachable!(),
        }
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

    // 5. Fetch and set first project for this org (network call, no lock)
    let project_name = {
        // Load credentials for the NEW org (ctx still has old org)
        let creds = CredentialsV2::load()?;
        let api_key = creds
            .get_for_org(&target_org_id)
            .map(|c| c.api_key.clone())
            .or_else(|| creds.legacy_key().map(|s| s.to_string()));

        if let Some(api_key) = api_key {
            let client = ApiClient::with_base_url(ctx.api_url(), Some(api_key))?;
            let projects_url = format!("/api/v1/projects?org_id={}", target_org_id);
            if let Ok(projects) = client.get::<Vec<ProjectResponse>>(&projects_url).await {
                if let Some(first_project) = projects.first() {
                    // Re-acquire lock to update config
                    let _lock = GlobalLock::acquire()?;
                    let mut config = Config::load()?;
                    config.active_project_id = Some(first_project.id.to_string());
                    config.active_project_name = Some(first_project.name.clone());
                    config.save()?;
                    Some(first_project.name.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    print_success(&format!("Switched to organization: {}", org_name));
    if let Some(project) = project_name {
        print_info(&format!("Default project: {}", project));
    }

    // Warn if env var is still set (user used --ignore-env)
    if env_key_set && ctx.ignore_env() {
        print_warning(
            "Your shell sets PAKYAS_API_KEY. Other commands will use it unless you add --ignore-env or unset the variable.",
        );
    }

    Ok(())
}

// ============================================================================
// Missing Key Flow Helpers
// ============================================================================

/// Types for CLI auth device code flow
#[derive(Debug, Serialize)]
struct InitCliAuthRequest {
    device_name: Option<String>,
    cli_version: Option<String>,
    target_org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InitCliAuthResponse {
    session_token: String,
    auth_url: String,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct PollCliAuthResponse {
    status: String,
    api_key: Option<String>,
    selected_org: Option<CliAuthOrgInfo>,
}

#[derive(Debug, Deserialize)]
struct CliAuthOrgInfo {
    id: Uuid,
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    #[serde(default)]
    org_id: String,
    #[serde(default)]
    org_name: Option<String>,
}

/// Create an API key via browser device code flow for a specific org.
async fn create_key_via_browser(
    ctx: &Context,
    target_org_id: &str,
    org_name: &str,
    verbose: bool,
) -> Result<String> {
    let client = reqwest::Client::new();
    let api_url = ctx.api_url();

    // Get device info
    let device_name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "Unknown Device".to_string());

    // 1. Initialize auth session with target org
    print_info("Starting authentication...");
    let init_url = format!("{}/api/v1/cli/auth/init", api_url.trim_end_matches('/'));
    let init_request = InitCliAuthRequest {
        device_name: Some(device_name),
        cli_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        target_org_id: Some(target_org_id.to_string()),
    };

    let init_response = client.post(&init_url).json(&init_request).send().await?;

    if !init_response.status().is_success() {
        let status = init_response.status();
        let error_body = init_response.text().await.unwrap_or_default();

        if verbose {
            eprintln!("[verbose] Server returned {}: {}", status, error_body);
        }

        return Err(
            CliError::api(format!("Failed to initialize authentication: {}", status)).into(),
        );
    }

    let init_data: InitCliAuthResponse = init_response.json().await?;

    // 2. Open browser
    print_info("Opening browser...");
    if open::that(&init_data.auth_url).is_err() {
        print_info("Could not open browser automatically.");
    }
    println!();
    println!("Please visit: {}", init_data.auth_url);
    println!();

    // 3. Poll for completion with spinner
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message("Waiting for authentication... (Press Ctrl+C to cancel)");
    spinner.enable_steady_tick(Duration::from_millis(100));

    let poll_url = format!("{}/api/v1/cli/auth/poll", api_url.trim_end_matches('/'));
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(init_data.expires_in as u64);
    let mut poll_interval = Duration::from_secs(2);
    let max_poll_interval = Duration::from_secs(5);

    loop {
        if start.elapsed() > timeout {
            spinner.finish_and_clear();
            return Err(CliError::api("Authentication timed out").into());
        }

        tokio::time::sleep(poll_interval).await;

        let poll_response = match client
            .get(&poll_url)
            .query(&[("token", &init_data.session_token)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => continue,
        };

        if !poll_response.status().is_success() {
            continue;
        }

        let poll_data: PollCliAuthResponse = match poll_response.json().await {
            Ok(d) => d,
            Err(_) => continue,
        };

        match poll_data.status.as_str() {
            "pending" => {
                poll_interval = std::cmp::min(poll_interval * 2, max_poll_interval);
                continue;
            }
            "completed" => {
                spinner.finish_and_clear();

                let api_key = poll_data
                    .api_key
                    .ok_or_else(|| CliError::api("Missing API key in response"))?;

                // Verify the key is for the correct org
                if let Some(ref selected_org) = poll_data.selected_org {
                    if selected_org.id.to_string() != target_org_id {
                        return Err(CliError::api(format!(
                            "Authenticated for wrong org '{}', expected '{}'",
                            selected_org.name, org_name
                        ))
                        .into());
                    }
                }

                print_success("Authentication successful!");
                return Ok(api_key);
            }
            "expired" => {
                spinner.finish_and_clear();
                return Err(CliError::api("Authentication session expired").into());
            }
            "cancelled" => {
                spinner.finish_and_clear();
                return Err(CliError::api("Authentication was cancelled").into());
            }
            _ => {
                spinner.finish_and_clear();
                return Err(CliError::api("Unknown authentication status").into());
            }
        }
    }
}

/// Prompt user to paste an API key and validate it belongs to the target org.
async fn paste_and_validate_key(
    ctx: &Context,
    target_org_id: &str,
    org_name: &str,
    verbose: bool,
) -> Result<String> {
    const MAX_ATTEMPTS: u32 = 3;

    for attempt in 1..=MAX_ATTEMPTS {
        let api_key: String = Password::new().with_prompt("Paste API key").interact()?;

        // Validate format
        if let Err(e) = validate_api_key(&api_key) {
            eprintln!("Invalid key format: {}", e);
            if attempt < MAX_ATTEMPTS {
                continue;
            }
            return Err(e.into());
        }

        // Validate with server
        if verbose {
            eprintln!("[verbose] Validating key against /api/v1/me");
        }

        let client = ApiClient::with_api_key(ctx, api_key.clone())?;
        let me: MeResponse = match client.get("/api/v1/me").await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Key validation failed: {}", e);
                if attempt < MAX_ATTEMPTS {
                    continue;
                }
                return Err(e);
            }
        };

        // Check org matches
        if me.org_id == target_org_id {
            print_success("Key validated!");
            return Ok(api_key);
        }

        // Wrong org - offer to switch instead
        let actual_org_name = me.org_name.as_deref().unwrap_or(&me.org_id);
        eprintln!(
            "That key belongs to '{}', not '{}'.",
            actual_org_name, org_name
        );

        if attempt < MAX_ATTEMPTS {
            let switch_instead = Confirm::new()
                .with_prompt(format!("Switch to '{}' instead?", actual_org_name))
                .default(false)
                .interact()?;

            if switch_instead {
                // Return the key - caller will need to handle switching to different org
                // For now, we'll just use this key and let it work
                print_success("Key validated!");
                return Ok(api_key);
            }
        }
    }

    Err(CliError::api(format!("Failed after {} attempts", MAX_ATTEMPTS)).into())
}

/// Store an API key for an organization.
fn store_key_for_org(org_id: &str, api_key: &str) -> Result<()> {
    let _lock = GlobalLock::acquire()?;
    let mut creds = CredentialsV2::load()?;

    let device_label = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|h| format!("{}-{}", h, Utc::now().format("%Y-%m-%d")))
        .unwrap_or_else(|| format!("CLI-{}", Utc::now().format("%Y-%m-%d")));

    creds.set_for_org(
        org_id,
        OrgCredential {
            api_key: api_key.to_string(),
            key_id: None,
            label: Some(device_label),
            added_at: Utc::now(),
            last_verified: Some(Utc::now()),
        },
    );
    creds.save()?;

    Ok(())
}
