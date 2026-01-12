use crate::cli::LoginArgs;
use crate::client::ApiClient;
use crate::config::{Config, Context};
use crate::credentials::{validate_api_key, Credentials, CredentialsV2, OrgCredential};
use crate::error::CliError;
use crate::output::{print_error, print_info, print_success, print_warning};
use anyhow::Result;
use chrono::Utc;
use dialoguer::{Input, Password};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    user: AuthUser,
    access_token: String,
    #[allow(dead_code)]
    refresh_token: String,
    #[allow(dead_code)]
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct AuthUser {
    id: Uuid,
    #[allow(dead_code)]
    email: String,
    name: String,
    #[allow(dead_code)]
    email_verified: bool,
}

#[derive(Debug, Serialize)]
struct CreateApiKeyRequest {
    org_id: Uuid,
    name: String,
    scopes: Vec<String>,
    expires_in_days: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ApiKeyCreated {
    #[allow(dead_code)]
    id: Uuid,
    #[allow(dead_code)]
    name: String,
    full_key: String,
    #[allow(dead_code)]
    key_prefix: String,
}

#[derive(Debug, Deserialize)]
struct OrganizationResponse {
    id: Uuid,
    name: String,
    #[allow(dead_code)]
    user_role: String,
}

#[derive(Debug, Deserialize)]
struct ProjectResponse {
    id: Uuid,
    #[allow(dead_code)]
    org_id: Uuid,
    name: String,
}

// CLI Auth types
#[derive(Debug, Serialize)]
struct InitCliAuthRequest {
    device_name: Option<String>,
    cli_version: Option<String>,
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
    user_email: Option<String>,
    user_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct CliAuthOrgInfo {
    id: Uuid,
    name: String,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Print a comprehensive login summary.
fn print_login_summary(
    email: Option<&str>,
    org_name: Option<&str>,
    project_name: Option<&str>,
    api_key: Option<&str>,
) {
    println!();
    print_success("Login successful!");
    println!();

    if let Some(email) = email {
        println!("  Email:        {}", email);
    }
    if let Some(org_name) = org_name {
        println!("  Organization: {}", org_name);
    }
    if let Some(project_name) = project_name {
        println!("  Project:      {}", project_name);
    } else {
        println!("  Project:      (none)");
    }
    if let Some(api_key) = api_key {
        let prefix = if api_key.len() > 12 {
            &api_key[..12]
        } else {
            api_key
        };
        println!("  API Key:      {}...", prefix);
    }

    println!();
    print_info("Run 'pakyas check create' to create your first check");
}

// ============================================================================
// Commands
// ============================================================================

/// Login to Pakyas.
///
/// Default: Opens browser for authentication.
/// With --api-key: Uses provided API key directly.
/// With --no-browser: Interactive email/password login.
pub async fn login(ctx: &Context, args: LoginArgs, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
    }

    // If API key provided directly, validate and store it
    if let Some(api_key) = args.api_key {
        if verbose {
            eprintln!("[verbose] Using API key authentication");
        }
        return login_with_api_key(ctx, &api_key).await;
    }

    // If --no-browser flag, use interactive email/password
    if args.no_browser {
        if verbose {
            eprintln!("[verbose] Using interactive (no-browser) authentication");
        }
        return login_interactive(ctx).await;
    }

    // Default: browser-based OAuth flow
    if verbose {
        eprintln!("[verbose] Using browser-based authentication");
    }
    login_with_browser(ctx, verbose).await
}

/// Login with an API key directly.
async fn login_with_api_key(ctx: &Context, api_key: &str) -> Result<()> {
    validate_api_key(api_key)?;

    // Test the API key by fetching organizations
    let client = ApiClient::with_api_key(ctx, api_key.to_string())?;
    let orgs: Vec<OrganizationResponse> = client.get("/api/v1/organizations").await?;

    if orgs.is_empty() {
        return Err(CliError::api("No organizations found for this API key").into());
    }

    // Select the first org (API key is bound to one org anyway)
    let selected_org = &orgs[0];
    let org_id = selected_org.id.to_string();

    // Save credentials in V2 format
    let mut creds_v2 = CredentialsV2::load()?;
    let device_label = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|h| format!("{}-{}", h, Utc::now().format("%Y-%m-%d")))
        .unwrap_or_else(|| format!("CLI-{}", Utc::now().format("%Y-%m-%d")));

    creds_v2.set_for_org(
        &org_id,
        OrgCredential {
            api_key: api_key.to_string(),
            key_id: None,
            label: Some(device_label),
            added_at: Utc::now(),
            last_verified: Some(Utc::now()),
        },
    );
    creds_v2.save()?;

    // Also save legacy format for backward compatibility
    let creds = Credentials {
        api_key: Some(api_key.to_string()),
        user_email: None,
        user_id: None,
    };
    creds.save()?;

    // Set default org and project if not already set
    let mut config = Config::load()?;
    if config.active_org_id.is_none() {
        config.active_org_id = Some(org_id);
        config.active_org_name = Some(selected_org.name.clone());

        // Clear stale project data first
        config.active_project_id = None;
        config.active_project_name = None;

        // Fetch and set the first project for this org
        let projects_url = format!("/api/v1/projects?org_id={}", selected_org.id);
        if let Ok(projects) = client.get::<Vec<ProjectResponse>>(&projects_url).await {
            if let Some(first_project) = projects.first() {
                config.active_project_id = Some(first_project.id.to_string());
                config.active_project_name = Some(first_project.name.clone());
            }
        }

        config.save()?;
    }

    print_login_summary(
        None,
        config.active_org_name.as_deref(),
        config.active_project_name.as_deref(),
        Some(api_key),
    );

    Ok(())
}

/// Interactive login with email and password.
async fn login_interactive(ctx: &Context) -> Result<()> {
    // Prompt for email
    let email: String = Input::new().with_prompt("Email").interact_text()?;

    // Prompt for password (hidden input)
    let password: String = Password::new().with_prompt("Password").interact()?;

    // Create temporary client for auth (no API key needed for login)
    let client = reqwest::Client::new();
    let api_url = ctx.api_url();
    let login_url = format!("{}/api/v1/auth/login", api_url.trim_end_matches('/'));

    let login_req = LoginRequest {
        email: email.clone(),
        password,
    };
    let response = client.post(&login_url).json(&login_req).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CliError::api("Invalid email or password").into());
        }
        return Err(CliError::api(format!("Login failed: {}", status)).into());
    }

    let auth_response: AuthResponse = response.json().await?;
    let access_token = auth_response.access_token;
    let user = auth_response.user;

    print_success(&format!("Authenticated as {}", user.name));

    // Fetch organizations using JWT
    let orgs_url = format!("{}/api/v1/organizations", api_url.trim_end_matches('/'));
    let orgs_response = client
        .get(&orgs_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;

    if !orgs_response.status().is_success() {
        return Err(CliError::api("Failed to fetch organizations").into());
    }

    let orgs: Vec<OrganizationResponse> = orgs_response.json().await?;

    if orgs.is_empty() {
        // No orgs yet, save just the user info for now
        let creds = Credentials {
            api_key: None,
            user_email: Some(email),
            user_id: Some(user.id.to_string()),
        };
        creds.save()?;
        print_info("No organizations found. Create one at https://pakyas.com");
        return Ok(());
    }

    // Select organization (use first one for simplicity, or could prompt)
    let selected_org = &orgs[0];
    print_info(&format!("Using organization: {}", selected_org.name));

    // Create an API key for CLI usage
    let api_key_url = format!("{}/api/v1/api-keys", api_url.trim_end_matches('/'));
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());

    let api_key_req = CreateApiKeyRequest {
        org_id: selected_org.id,
        name: format!("CLI - {}", hostname),
        scopes: vec!["read".to_string(), "write".to_string()],
        expires_in_days: None, // No expiration
    };

    let api_key_response = client
        .post(&api_key_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&api_key_req)
        .send()
        .await?;

    if !api_key_response.status().is_success() {
        // If we can't create an API key, the user may not have permission
        // Fall back to storing nothing and informing the user
        print_error("Could not create API key. You may need to create one manually.");
        print_info("Use: pakyas login --api-key <KEY>");
        return Ok(());
    }

    let api_key: ApiKeyCreated = api_key_response.json().await?;

    // Save credentials
    let creds = Credentials {
        api_key: Some(api_key.full_key.clone()),
        user_email: Some(email.clone()),
        user_id: Some(user.id.to_string()),
    };
    creds.save()?;

    // Save config with active org and project
    let mut config = Config::load()?;
    config.active_org_id = Some(selected_org.id.to_string());
    config.active_org_name = Some(selected_org.name.clone());

    // Clear stale project data first
    config.active_project_id = None;
    config.active_project_name = None;

    // Fetch and set the first project for this org
    let api_client = ApiClient::with_api_key(ctx, api_key.full_key.clone())?;
    let projects_url = format!("/api/v1/projects?org_id={}", selected_org.id);
    if let Ok(projects) = api_client.get::<Vec<ProjectResponse>>(&projects_url).await {
        if let Some(first_project) = projects.first() {
            config.active_project_id = Some(first_project.id.to_string());
            config.active_project_name = Some(first_project.name.clone());
        }
    }

    config.save()?;

    print_login_summary(
        Some(&email),
        Some(&selected_org.name),
        config.active_project_name.as_deref(),
        Some(&api_key.full_key),
    );

    Ok(())
}

/// Browser-based login flow.
///
/// Opens a browser for authentication and polls for completion.
async fn login_with_browser(ctx: &Context, verbose: bool) -> Result<()> {
    let client = reqwest::Client::new();
    let api_url = ctx.api_url();

    // Get device info
    let device_name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "Unknown Device".to_string());

    // 1. Initialize auth session
    print_info("Starting authentication...");
    let init_url = format!("{}/api/v1/cli/auth/init", api_url.trim_end_matches('/'));
    let init_request = InitCliAuthRequest {
        device_name: Some(device_name.clone()),
        cli_version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };

    let init_response = client.post(&init_url).json(&init_request).send().await?;

    if !init_response.status().is_success() {
        let status = init_response.status();
        let error_body = init_response.text().await.unwrap_or_default();

        if verbose {
            eprintln!("[verbose] Server returned {}: {}", status, error_body);
        }

        // Try to parse as JSON error response
        let error_msg = serde_json::from_str::<serde_json::Value>(&error_body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| format!("Server returned {}", status));

        return Err(CliError::api(format!("Failed to initialize authentication: {}", error_msg)).into());
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
        // Check timeout
        if start.elapsed() > timeout {
            spinner.finish_and_clear();
            return Err(CliError::api("Authentication timed out").into());
        }

        // Wait before polling
        tokio::time::sleep(poll_interval).await;

        // Poll
        let poll_response = client
            .get(&poll_url)
            .query(&[("token", &init_data.session_token)])
            .send()
            .await;

        let poll_response = match poll_response {
            Ok(r) => r,
            Err(_) => {
                // Network error, retry
                continue;
            }
        };

        if !poll_response.status().is_success() {
            // Server error, retry
            continue;
        }

        let poll_data: PollCliAuthResponse = match poll_response.json().await {
            Ok(d) => d,
            Err(_) => continue,
        };

        match poll_data.status.as_str() {
            "pending" => {
                // Exponential backoff up to max_poll_interval
                poll_interval = std::cmp::min(poll_interval * 2, max_poll_interval);
                continue;
            }
            "completed" => {
                spinner.finish_and_clear();

                let api_key = poll_data
                    .api_key
                    .ok_or_else(|| CliError::api("Missing API key in response"))?;

                let user_email = poll_data.user_email.clone();

                // Save credentials in V2 format (per-org storage)
                if let Some(ref org) = poll_data.selected_org {
                    let mut creds = CredentialsV2::load()?;
                    let org_id = org.id.to_string();

                    // Create device label
                    let device_label = hostname::get()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                        .map(|h| format!("{}-{}", h, Utc::now().format("%Y-%m-%d")))
                        .unwrap_or_else(|| format!("CLI-{}", Utc::now().format("%Y-%m-%d")));

                    creds.set_for_org(
                        &org_id,
                        OrgCredential {
                            api_key: api_key.clone(),
                            key_id: None,
                            label: Some(device_label),
                            added_at: Utc::now(),
                            last_verified: Some(Utc::now()),
                        },
                    );
                    creds.save()?;

                    // Also save legacy format for backward compatibility
                    let legacy_creds = Credentials {
                        api_key: Some(api_key.clone()),
                        user_email: poll_data.user_email.clone(),
                        user_id: poll_data.user_id.map(|u| u.to_string()),
                    };
                    legacy_creds.save()?;

                    // Save config with org and project
                    let mut config = Config::load()?;
                    config.active_org_id = Some(org_id.clone());
                    config.active_org_name = Some(org.name.clone());

                    // Clear stale project data first
                    config.active_project_id = None;
                    config.active_project_name = None;

                    // Fetch and set the first project for this org
                    let api_client = ApiClient::with_api_key(ctx, api_key.clone())?;
                    let projects_url = format!("/api/v1/projects?org_id={}", org.id);
                    if let Ok(projects) =
                        api_client.get::<Vec<ProjectResponse>>(&projects_url).await
                    {
                        if let Some(first_project) = projects.first() {
                            config.active_project_id = Some(first_project.id.to_string());
                            config.active_project_name = Some(first_project.name.clone());
                        }
                    }

                    config.save()?;

                    print_login_summary(
                        user_email.as_deref(),
                        Some(&org.name),
                        config.active_project_name.as_deref(),
                        Some(&api_key),
                    );
                } else {
                    // No org selected - save as legacy credential
                    let creds = Credentials {
                        api_key: Some(api_key.clone()),
                        user_email: poll_data.user_email,
                        user_id: poll_data.user_id.map(|u| u.to_string()),
                    };
                    creds.save()?;

                    print_login_summary(user_email.as_deref(), None, None, Some(&api_key));
                }

                return Ok(());
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

/// Logout and clear credentials.
pub async fn logout(_ctx: &Context, verbose: bool) -> Result<()> {
    if verbose {
        if let Ok(path) = Credentials::path() {
            eprintln!("[verbose] Clearing credentials at: {}", path.display());
        }
    }
    Credentials::clear()?;
    print_success("Logged out successfully");
    Ok(())
}

/// Show current user and context.
pub async fn whoami(ctx: &Context, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Ok(path) = Credentials::path() {
            eprintln!("[verbose] Credentials path: {}", path.display());
        }
    }

    let creds = Credentials::load()?;

    if !creds.is_authenticated() {
        print_error("Not logged in");
        print_info("Run: pakyas login");
        return Ok(());
    }

    // Show user info if available
    if let Some(email) = &creds.user_email {
        println!("Email: {}", email);
    }

    // Show active org
    if let Some(org_name) = ctx.active_org_name() {
        println!("Organization: {}", org_name);
    } else if let Some(org_id) = ctx.active_org_id() {
        println!("Organization ID: {}", org_id);
    } else {
        println!("Organization: (none selected)");
    }

    // Show active project
    if let Some(project_name) = ctx.active_project_name() {
        println!("Project: {}", project_name);
    } else if let Some(project_id) = ctx.active_project_id() {
        println!("Project ID: {}", project_id);
    } else {
        println!("Project: (none selected)");
    }

    // Show API key prefix if available
    if let Some(api_key) = &creds.api_key {
        let prefix = if api_key.len() > 12 {
            &api_key[..12]
        } else {
            api_key
        };
        println!("API Key: {}...", prefix);
    }

    Ok(())
}

/// Show detailed authentication status.
///
/// Displays credential storage info, active org, key sources, and warnings.
pub async fn auth_status(ctx: &Context, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Ok(path) = CredentialsV2::path() {
            eprintln!("[verbose] Credentials path: {}", path.display());
        }
    }

    // Check if env var is set
    let env_key_set = std::env::var("PAKYAS_API_KEY").is_ok();

    // Load V2 credentials (ignoring env var to show actual stored state)
    let creds = load_credentials_ignoring_env()?;

    println!("Authentication Status");
    println!("=====================");
    println!();

    // Environment variable status
    if env_key_set {
        print_warning("PAKYAS_API_KEY environment variable is set");
        println!("  All API calls will use the env var key (overrides stored credentials)");
        println!("  Use --ignore-env to use stored credentials instead");
        println!();
    }

    // Active organization
    println!("Active Organization:");
    if let Some(org_name) = ctx.active_org_name() {
        println!("  Name: {}", org_name);
    }
    if let Some(org_id) = ctx.active_org_id() {
        println!("  ID:   {}", org_id);

        // Check if we have a key for this org
        if creds.has_key_for_org(org_id) {
            let cred = creds.get_for_org(org_id).unwrap();
            let key_preview = format_key_preview(&cred.api_key);
            println!("  Key:  {} (stored)", key_preview);
            if let Some(ref label) = cred.label {
                println!("  Label: {}", label);
            }
            if let Some(verified) = cred.last_verified {
                println!("  Last verified: {}", verified.format("%Y-%m-%d %H:%M:%S UTC"));
            }
        } else if !env_key_set {
            print_warning(&format!("No stored key for active org '{}'", org_id));
            println!("  Run 'pakyas auth key set --org {}' to add one", org_id);
        }
    } else {
        println!("  (none selected)");
        print_info("Run 'pakyas org switch <org>' to select an organization");
    }
    println!();

    // Active project
    println!("Active Project:");
    if let Some(project_name) = ctx.active_project_name() {
        println!("  Name: {}", project_name);
    }
    if let Some(project_id) = ctx.active_project_id() {
        println!("  ID:   {}", project_id);
    } else {
        println!("  (none selected)");
    }
    println!();

    // Stored credentials summary
    println!("Stored Credentials:");
    let org_count = creds.orgs.len();
    if org_count > 0 {
        println!("  {} organization(s) with stored keys", org_count);
        for org_id in creds.list_orgs_with_keys() {
            let cred = creds.get_for_org(org_id).unwrap();
            let key_preview = format_key_preview(&cred.api_key);
            println!("    - {}: {}", org_id, key_preview);
        }
    } else {
        println!("  No per-org keys stored");
    }

    // Legacy key warning
    if creds.has_legacy_key() {
        println!();
        print_warning("Legacy API key detected (not associated with any org)");
        if let Some(legacy_key) = creds.legacy_key() {
            println!("  Key: {}", format_key_preview(legacy_key));
        }
        println!("  Run 'pakyas org switch <org>' to migrate it, or 'pakyas auth key rm --legacy' to remove.");
    }

    Ok(())
}

// ============================================================================
// Helper functions
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
