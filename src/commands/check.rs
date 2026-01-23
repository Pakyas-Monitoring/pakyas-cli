use crate::cache::{CheckCache, CheckLike};
use crate::cli::{CheckCommands, FailOnSeverity};
use crate::client::ApiClient;
use crate::config::Context;
use crate::cron::{effective_period_from_cron, next_cron_times_in_tz, validate_cron_expression};
use crate::error::CliError;
use crate::exit_codes;
use crate::output::{format_status, print_output, print_single, print_success, print_warning};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use dialoguer::{Confirm, Input, Select};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Check {
    pub id: Uuid,
    pub public_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub period_seconds: i32,
    pub grace_seconds: i32,
    #[serde(default)]
    pub schedule_type: String,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    pub status: String,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub next_ping_expected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    // Lateness/alerting fields
    #[serde(default)]
    pub alert_after_failures: Option<i32>,
    #[serde(default)]
    pub consecutive_failures: i32,
    #[serde(default = "default_late_after_ratio")]
    pub late_after_ratio: f32,
    #[serde(default)]
    pub max_runtime_seconds: Option<i32>,
    #[serde(default = "default_missed_before_alert")]
    pub missed_before_alert: i32,
    // Additional fields from API
    #[serde(default)]
    pub soft_deleted: bool,
    #[serde(default)]
    pub alert_on_down: Option<bool>,
    #[serde(default)]
    pub alert_on_late: Option<bool>,
    #[serde(default)]
    pub alert_on_overrun: Option<bool>,
    #[serde(default)]
    pub alert_on_anomaly: Option<bool>,
    #[serde(default)]
    pub anomaly_status_performance: Option<String>,
    #[serde(default)]
    pub anomaly_status_reliability: Option<String>,
    #[serde(default)]
    pub status_before_start: Option<String>,
    #[serde(default)]
    pub notify_on_recovery: Option<bool>,
}

fn default_late_after_ratio() -> f32 {
    0.2
}

fn default_missed_before_alert() -> i32 {
    1
}

impl CheckLike for Check {
    fn id(&self) -> Uuid {
        self.id
    }
    fn public_id(&self) -> Uuid {
        self.public_id
    }
    fn slug(&self) -> &str {
        &self.slug
    }
    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Serialize)]
struct CreateCheckRequest {
    project_id: Uuid,
    name: String,
    slug: String,
    period_seconds: i32,
    grace_seconds: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cron_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<String>,
}

#[derive(Debug, Serialize, Default)]
struct UpdateCheckRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cron_expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    period_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grace_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alert_after_failures: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    late_after_ratio: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_runtime_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missed_before_alert: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PingLog {
    pub id: i64,
    #[serde(rename = "type")]
    pub ping_type: String,
    pub created_at: DateTime<Utc>,
    pub duration_ms: Option<i32>,
    pub source_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PingHistoryResponse {
    pings: Vec<PingLog>,
    total: i64,
}

// ============================================================================
// Display Types
// ============================================================================

/// Check with project name for org-wide listing
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckWithProject {
    #[serde(flatten)]
    pub check: Check,
    pub project_name: String,
}

#[derive(Debug, Tabled, Serialize)]
struct CheckRowWithProject {
    #[tabled(rename = "PROJECT")]
    project: String,
    #[tabled(rename = "NAME")]
    name: String,
    #[tabled(rename = "SLUG")]
    slug: String,
    #[tabled(rename = "PUBLIC_ID")]
    public_id: String,
    #[tabled(rename = "STATUS")]
    status: String,
    #[tabled(rename = "PERIOD")]
    period: String,
    #[tabled(rename = "LAST PING")]
    last_ping: String,
}

#[derive(Debug, Tabled, Serialize)]
struct PingRow {
    #[tabled(rename = "TIME")]
    time: String,
    #[tabled(rename = "TYPE")]
    ping_type: String,
    #[tabled(rename = "DURATION")]
    duration: String,
    #[tabled(rename = "SOURCE")]
    source: String,
}

#[derive(Debug, Serialize)]
struct CheckDetail {
    id: String,
    public_id: String,
    name: String,
    slug: String,
    status: String,
    period: String,
    grace: String,
    description: Option<String>,
    tags: Vec<String>,
    last_ping: String,
    next_expected: String,
    ping_url: String,
    created_at: String,
}

// ============================================================================
// Commands
// ============================================================================

/// Handle check subcommands
pub async fn handle(ctx: &Context, command: CheckCommands, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Some(project) = ctx.active_project_name() {
            eprintln!("[verbose] Active project: {}", project);
        }
    }

    match command {
        CheckCommands::List { project } => list(ctx, project.as_deref(), verbose).await,
        CheckCommands::Create {
            slug,
            name,
            cron,
            tz,
            every,
            grace,
            description,
            json,
            quiet,
            dry_run,
            interactive,
        } => {
            create(
                ctx,
                slug,
                name,
                cron,
                tz,
                every,
                grace,
                description,
                json,
                quiet,
                dry_run,
                interactive,
                verbose,
            )
            .await
        }
        CheckCommands::Show { slug } => show(ctx, &slug, verbose).await,
        CheckCommands::Pause { slug } => pause(ctx, &slug, verbose).await,
        CheckCommands::Resume { slug } => resume(ctx, &slug, verbose).await,
        CheckCommands::Delete { slug, yes } => delete(ctx, &slug, yes, verbose).await,
        CheckCommands::Logs { slug, limit } => logs(ctx, &slug, limit, verbose).await,
        CheckCommands::Sync => sync(ctx, verbose).await,
        CheckCommands::Update {
            slug,
            name,
            description,
            cron,
            tz,
            every,
            grace,
            tags,
            alert_after_failures,
            late_after_ratio,
            max_runtime,
            missed_before_alert,
            yes,
        } => {
            update(
                ctx,
                &slug,
                name,
                description,
                cron,
                tz,
                every,
                grace,
                tags,
                alert_after_failures,
                late_after_ratio,
                max_runtime,
                missed_before_alert,
                yes,
                verbose,
            )
            .await
        }
        CheckCommands::Inspect { slug } => inspect(ctx, &slug, verbose).await,
        CheckCommands::Doctor {
            slug,
            deep,
            fail_on,
        } => doctor(ctx, &slug, deep, fail_on, verbose).await,
        CheckCommands::Tail {
            slug,
            since,
            types,
            follow,
            limit,
        } => tail(ctx, &slug, &since, types.as_deref(), follow, limit, verbose).await,
    }
}

/// List all checks in the organization (optionally filtered by project)
async fn list(ctx: &Context, project_filter: Option<&str>, verbose: bool) -> Result<()> {
    use crate::commands::project::resolve_project;

    let client = ApiClient::new(ctx)?;
    let org_id = ctx.require_org()?;

    // If project filter specified, resolve it and filter by project_id
    let url = if let Some(project_identifier) = project_filter {
        let project = resolve_project(ctx, project_identifier).await?;
        if verbose {
            eprintln!(
                "[verbose] Filtering to project: {} ({})",
                project.name, project.id
            );
        }
        format!("/api/v1/checks?project_id={}", project.id)
    } else {
        format!("/api/v1/checks?org_id={}", org_id)
    };

    if verbose {
        eprintln!("[verbose] Fetching checks from: {}", url);
    }

    let checks: Vec<CheckWithProject> = client.get(&url).await?;

    if verbose {
        eprintln!("[verbose] Found {} check(s)", checks.len());
    }

    // Update cache with org_id (for org-wide cache)
    let mut cache = CheckCache::load()?;
    cache.update_from_checks(org_id, checks.iter().map(|c| c.check.clone()));
    cache.save()?;

    let rows: Vec<CheckRowWithProject> = checks
        .into_iter()
        .map(|c| CheckRowWithProject {
            project: c.project_name,
            name: c.check.name,
            slug: c.check.slug,
            public_id: c.check.public_id.to_string(),
            status: format_status(&c.check.status),
            period: format_duration(c.check.period_seconds),
            last_ping: format_relative_time(c.check.last_ping_at),
        })
        .collect();

    print_output(ctx, rows)?;

    Ok(())
}

/// Create a new check
#[allow(clippy::too_many_arguments)]
async fn create(
    ctx: &Context,
    slug: String,
    name: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    every: Option<String>,
    grace: Option<String>,
    description: Option<String>,
    json_output: bool,
    quiet: bool,
    dry_run: bool,
    interactive: bool,
    _verbose: bool,
) -> Result<()> {
    let project_id = ctx.require_project()?;

    // Validate slug locally (matches backend SLUG_REGEX)
    validate_slug(&slug)?;

    // Determine mode
    let has_schedule = cron.is_some() || every.is_some();

    if !has_schedule && !interactive {
        return Err(anyhow!(
            "Missing schedule. Use one of:\n\n\
             Cron schedule:\n\
               pakyas check create {} --cron \"0 2 * * *\"\n\n\
             Interval:\n\
               pakyas check create {} --every 5m\n\n\
             Interactive:\n\
               pakyas check create {} -i",
            slug,
            slug,
            slug
        ));
    }

    // Handle interactive mode
    if interactive {
        return create_interactive(ctx, slug, name, description).await;
    }

    // Parse timezone (clap ensures --tz only with --cron)
    let parsed_tz: Option<chrono_tz::Tz> = if let Some(tz_str) = &tz {
        Some(validate_timezone(tz_str)?)
    } else {
        None
    };

    // Validate and prepare schedule
    let (cron_expression, timezone, period_seconds, grace_seconds, grace_auto) =
        if let Some(cron_expr) = &cron {
            validate_cron_cli(cron_expr)?;

            let period = effective_period_from_cron(cron_expr).unwrap_or(3600);

            let (grace_val, auto) = if let Some(g) = &grace {
                (parse_duration(g)?, false)
            } else {
                (smart_grace(period), true)
            };

            (Some(cron_expr.clone()), tz.clone(), period, grace_val, auto)
        } else {
            let every_str = every.as_ref().unwrap();
            let period = parse_duration(every_str)?;

            let (grace_val, auto) = if let Some(g) = &grace {
                (parse_duration(g)?, false)
            } else {
                (smart_grace(period), true)
            };

            (None, None, period, grace_val, auto)
        };

    // Dry run output
    if dry_run {
        // Compute effective timezone and source
        let (effective_tz, tz_source) = if let Some(tz) = parsed_tz {
            (tz, "check")
        } else if let Some(org_tz) = ctx
            .config
            .active_org_timezone
            .as_ref()
            .and_then(|s| s.parse().ok())
        {
            (org_tz, "org")
        } else {
            (chrono_tz::UTC, "utc_fallback")
        };

        print_dry_run(
            &slug,
            &name,
            &cron_expression,
            period_seconds,
            grace_seconds,
            grace_auto,
            effective_tz,
            tz_source,
        );
        return Ok(());
    }

    // Build request
    let display_name = name.unwrap_or_else(|| slug_to_title(&slug));
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| CliError::Other("Invalid project ID".to_string()))?;

    let req = CreateCheckRequest {
        project_id: project_uuid,
        name: display_name.clone(),
        slug: slug.clone(),
        period_seconds,
        grace_seconds,
        description,
        cron_expression,
        timezone,
    };

    // Create check
    let client = ApiClient::new(ctx)?;
    let check: Check = client.post("/api/v1/checks", &req).await?;

    // Update cache
    let mut cache = CheckCache::load()?;
    cache.set(
        project_id,
        &check.slug,
        check.id,
        check.public_id,
        check.name.clone(),
    );
    cache.save()?;

    // Output
    let ping_url = format!("{}/{}", ctx.ping_url(), check.public_id);

    if quiet {
        println!("{}", ping_url);
    } else if json_output {
        print_check_json(ctx, &check, &ping_url, grace_auto);
    } else {
        print_check_created(ctx, &check, &ping_url, grace_auto);
    }

    Ok(())
}

/// Interactive check creation mode
async fn create_interactive(
    ctx: &Context,
    slug: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<()> {
    let project_id = ctx.require_project()?;

    // Name (default from slug)
    let display_name = match name {
        Some(n) => n,
        None => Input::new()
            .with_prompt("Display name")
            .default(slug_to_title(&slug))
            .interact_text()?,
    };

    // Schedule type
    let schedule_type = Select::new()
        .with_prompt("Schedule type")
        .items(&[
            "Cron expression (e.g., daily at 2am)",
            "Interval (e.g., every 5 minutes)",
        ])
        .default(0)
        .interact()?;

    let (cron_expression, timezone, period_seconds) = if schedule_type == 0 {
        // Cron
        let cron_expr: String = Input::new()
            .with_prompt("Cron expression (5-field)")
            .with_initial_text("0 2 * * *")
            .interact_text()?;

        validate_cron_cli(&cron_expr)?;

        let tz_input: String = Input::new()
            .with_prompt("Timezone (IANA format, or blank for org default)")
            .allow_empty(true)
            .interact_text()?;

        let tz = if tz_input.is_empty() {
            None
        } else {
            validate_timezone(&tz_input)?;
            Some(tz_input)
        };

        let period = effective_period_from_cron(&cron_expr).unwrap_or(3600);
        (Some(cron_expr), tz, period)
    } else {
        // Interval
        let every_input: String = Input::new()
            .with_prompt("Interval (e.g., 5m, 1h)")
            .interact_text()?;

        let period = parse_duration(&every_input)?;
        (None, None, period)
    };

    // Grace period (blank = auto)
    let default_grace = smart_grace(period_seconds);
    let grace_input: String = Input::new()
        .with_prompt(format!(
            "Grace period (blank = auto) [auto: {}]",
            format_duration(default_grace)
        ))
        .allow_empty(true)
        .interact_text()?;

    let (grace_seconds, grace_auto) = if grace_input.trim().is_empty() {
        (default_grace, true)
    } else {
        (parse_duration(&grace_input)?, false)
    };

    // Description
    let final_description = match description {
        Some(d) => Some(d),
        None => {
            let desc: String = Input::new()
                .with_prompt("Description (optional)")
                .allow_empty(true)
                .interact_text()?;
            if desc.is_empty() { None } else { Some(desc) }
        }
    };

    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| CliError::Other("Invalid project ID".to_string()))?;

    // Create
    let req = CreateCheckRequest {
        project_id: project_uuid,
        name: display_name,
        slug: slug.clone(),
        period_seconds,
        grace_seconds,
        description: final_description,
        cron_expression,
        timezone,
    };

    let client = ApiClient::new(ctx)?;
    let check: Check = client.post("/api/v1/checks", &req).await?;

    // Update cache
    let mut cache = CheckCache::load()?;
    cache.set(
        project_id,
        &check.slug,
        check.id,
        check.public_id,
        check.name.clone(),
    );
    cache.save()?;

    let ping_url = format!("{}/{}", ctx.ping_url(), check.public_id);
    print_check_created(ctx, &check, &ping_url, grace_auto);

    Ok(())
}

/// Show check details
async fn show(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;

    let detail = CheckDetail {
        id: check.id.to_string(),
        public_id: check.public_id.to_string(),
        name: check.name,
        slug: check.slug,
        status: check.status,
        period: format_duration(check.period_seconds),
        grace: format_duration(check.grace_seconds),
        description: check.description,
        tags: check.tags,
        last_ping: format_relative_time(check.last_ping_at),
        next_expected: format_relative_time(check.next_ping_expected_at),
        ping_url: format!("{}/{}", ctx.ping_url(), check.public_id),
        created_at: check.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    };

    print_single(ctx, &detail)?;

    Ok(())
}

/// Set check state (pause or resume)
async fn set_check_state(ctx: &Context, slug_or_id: &str, action: &str) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/checks/{}/{}", check.id, action);
    client.patch_no_response(&url).await?;

    let verb = if action == "pause" {
        "Paused"
    } else {
        "Resumed"
    };
    print_success(&format!("{} check: {}", verb, check.name));

    Ok(())
}

/// Pause a check
async fn pause(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    set_check_state(ctx, slug_or_id, "pause").await
}

/// Resume a paused check
async fn resume(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    set_check_state(ctx, slug_or_id, "resume").await
}

/// Delete a check
async fn delete(ctx: &Context, slug_or_id: &str, skip_confirm: bool, _verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;

    if !skip_confirm {
        let confirm = Confirm::new()
            .with_prompt(format!(
                "Delete check '{}'? This cannot be undone",
                check.name
            ))
            .default(false)
            .interact()?;

        if !confirm {
            print_warning("Cancelled");
            return Ok(());
        }
    }

    let client = ApiClient::new(ctx)?;
    let url = format!("/api/v1/checks/{}", check.id);
    client.delete(&url).await?;

    // Invalidate cache
    let mut cache = CheckCache::load()?;
    cache.invalidate(org_id, &check.slug);
    cache.save()?;

    print_success(&format!("Deleted check: {}", check.name));

    Ok(())
}

/// Show ping history for a check
async fn logs(ctx: &Context, slug_or_id: &str, limit: i32, _verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/checks/{}/pings?limit={}", check.id, limit);
    let response: PingHistoryResponse = client.get(&url).await?;

    let rows: Vec<PingRow> = response
        .pings
        .into_iter()
        .map(|p| PingRow {
            time: format_relative_time(Some(p.created_at)),
            ping_type: format_ping_type(&p.ping_type),
            duration: p
                .duration_ms
                .map(|d| format!("{}ms", d))
                .unwrap_or_else(|| "-".to_string()),
            source: p.source_ip.unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    if rows.is_empty() {
        print_warning("No pings recorded yet");
    } else {
        print_output(ctx, rows)?;
        println!("\nTotal: {} pings", response.total);
    }

    Ok(())
}

/// Force refresh the local check cache for the organization
async fn sync(ctx: &Context, _verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let client = ApiClient::new(ctx)?;

    let url = format!("/api/v1/checks?org_id={}", org_id);
    let checks: Vec<CheckWithProject> = client.get(&url).await?;

    // Clear and rebuild cache for this org
    let mut cache = CheckCache::load()?;
    cache.clear_project(org_id);
    cache.update_from_checks(org_id, checks.iter().map(|c| c.check.clone()));
    cache.save()?;

    print_success(&format!("Synced {} checks", checks.len()));

    Ok(())
}

/// Update a check's configuration
#[allow(clippy::too_many_arguments)]
async fn update(
    ctx: &Context,
    slug_or_id: &str,
    name: Option<String>,
    description: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    every: Option<String>,
    grace: Option<String>,
    tags: Option<String>,
    alert_after_failures: Option<i32>,
    late_after_ratio: Option<f32>,
    max_runtime: Option<String>,
    missed_before_alert: Option<i32>,
    skip_confirm: bool,
    _verbose: bool,
) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    // Check if any options were provided (non-interactive mode)
    let has_options = name.is_some()
        || description.is_some()
        || cron.is_some()
        || tz.is_some()
        || every.is_some()
        || grace.is_some()
        || tags.is_some()
        || alert_after_failures.is_some()
        || late_after_ratio.is_some()
        || max_runtime.is_some()
        || missed_before_alert.is_some();

    let req = if has_options {
        // Non-interactive mode: use provided options
        build_update_request_from_options(
            name,
            description,
            cron,
            tz,
            every,
            grace,
            tags,
            alert_after_failures,
            late_after_ratio,
            max_runtime,
            missed_before_alert,
        )?
    } else {
        // Interactive mode: prompt for each field
        build_update_request_interactive(&check)?
    };

    // Check if there are any changes
    if req.name.is_none()
        && req.description.is_none()
        && req.cron_expression.is_none()
        && req.timezone.is_none()
        && req.period_seconds.is_none()
        && req.grace_seconds.is_none()
        && req.tags.is_none()
        && req.alert_after_failures.is_none()
        && req.late_after_ratio.is_none()
        && req.max_runtime_seconds.is_none()
        && req.missed_before_alert.is_none()
    {
        print_warning("No changes specified");
        return Ok(());
    }

    // Show changes
    println!("\nChanges to '{}':", check.name);
    if let Some(ref new_name) = req.name {
        println!("  Name: {} → {}", check.name, new_name);
    }
    if let Some(ref new_desc) = req.description {
        let old_desc = check.description.as_deref().unwrap_or("(none)");
        let new_desc_display = if new_desc.is_empty() {
            "(cleared)"
        } else {
            new_desc
        };
        println!("  Description: {} → {}", old_desc, new_desc_display);
    }
    if let Some(ref new_cron) = req.cron_expression {
        let old_cron = check.cron_expression.as_deref().unwrap_or("(none)");
        let new_cron_display = if new_cron.is_empty() {
            "(cleared - switching to interval)"
        } else {
            new_cron
        };
        println!("  Cron: {} → {}", old_cron, new_cron_display);
    }
    if let Some(ref new_tz) = req.timezone {
        let old_tz = check.timezone.as_deref().unwrap_or("(org default)");
        let new_tz_display = if new_tz.is_empty() {
            "(org default)"
        } else {
            new_tz
        };
        println!("  Timezone: {} → {}", old_tz, new_tz_display);
    }
    if let Some(new_period) = req.period_seconds {
        println!(
            "  Period: {} → {}",
            format_duration(check.period_seconds),
            format_duration(new_period)
        );
    }
    if let Some(new_grace) = req.grace_seconds {
        println!(
            "  Grace: {} → {}",
            format_duration(check.grace_seconds),
            format_duration(new_grace)
        );
    }
    if let Some(ref new_tags) = req.tags {
        let old_tags = if check.tags.is_empty() {
            "(none)".to_string()
        } else {
            check.tags.join(", ")
        };
        let new_tags_display = if new_tags.is_empty() {
            "(cleared)".to_string()
        } else {
            new_tags.join(", ")
        };
        println!("  Tags: {} → {}", old_tags, new_tags_display);
    }
    if let Some(new_aaf) = req.alert_after_failures {
        println!(
            "  Alert after failures: {} → {}",
            check
                .alert_after_failures
                .map(|v| v.to_string())
                .unwrap_or_else(|| "inherited".to_string()),
            new_aaf
        );
    }
    if let Some(new_lar) = req.late_after_ratio {
        println!(
            "  Late after ratio: {:.0}% → {:.0}%",
            check.late_after_ratio * 100.0,
            new_lar * 100.0
        );
    }
    if let Some(new_max_runtime) = req.max_runtime_seconds {
        let old_max = check
            .max_runtime_seconds
            .map(format_duration)
            .unwrap_or_else(|| "(none)".to_string());
        println!(
            "  Max runtime: {} → {}",
            old_max,
            format_duration(new_max_runtime)
        );
    }
    if let Some(new_mba) = req.missed_before_alert {
        println!(
            "  Missed before alert: {} → {}",
            check.missed_before_alert, new_mba
        );
    }

    // Confirm
    if !skip_confirm {
        let confirm = Confirm::new()
            .with_prompt("Apply these changes?")
            .default(true)
            .interact()?;

        if !confirm {
            print_warning("Cancelled");
            return Ok(());
        }
    }

    // Send update request
    let url = format!("/api/v1/checks/{}", check.id);
    client.put_no_response(&url, &req).await?;

    print_success(&format!("Updated check: {}", check.name));

    Ok(())
}

/// Build UpdateCheckRequest from CLI options (non-interactive mode)
#[allow(clippy::too_many_arguments)]
fn build_update_request_from_options(
    name: Option<String>,
    description: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    every: Option<String>,
    grace: Option<String>,
    tags: Option<String>,
    alert_after_failures: Option<i32>,
    late_after_ratio: Option<f32>,
    max_runtime: Option<String>,
    missed_before_alert: Option<i32>,
) -> Result<UpdateCheckRequest> {
    // Handle cron validation
    let cron_expression = if let Some(ref c) = cron {
        if c.is_empty() {
            Some(String::new()) // Clear cron
        } else {
            validate_cron_cli(c)?;
            Some(c.clone())
        }
    } else {
        None
    };

    // Handle timezone validation
    let timezone = if let Some(ref t) = tz {
        if t.is_empty() {
            Some(String::new()) // Clear timezone
        } else {
            validate_timezone(t)?;
            Some(t.clone())
        }
    } else {
        None
    };

    let period_seconds = every.map(|p| parse_duration(&p)).transpose()?;
    let grace_seconds = grace.map(|g| parse_duration(&g)).transpose()?;
    let max_runtime_seconds = max_runtime.map(|m| parse_duration(&m)).transpose()?;
    let tags_vec = tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    Ok(UpdateCheckRequest {
        name,
        description,
        cron_expression,
        timezone,
        period_seconds,
        grace_seconds,
        tags: tags_vec,
        alert_after_failures,
        late_after_ratio,
        max_runtime_seconds,
        missed_before_alert,
    })
}

/// Build UpdateCheckRequest interactively by prompting for each field
fn build_update_request_interactive(check: &Check) -> Result<UpdateCheckRequest> {
    println!("\nUpdating check: {}", check.name);
    println!("Press Enter to keep current value, or enter new value.\n");

    let mut req = UpdateCheckRequest::default();

    // Name
    let name_input: String = Input::new()
        .with_prompt("Name")
        .default(check.name.clone())
        .interact_text()?;
    if name_input != check.name {
        req.name = Some(name_input);
    }

    // Description
    let current_desc = check.description.clone().unwrap_or_default();
    let desc_input: String = Input::new()
        .with_prompt("Description (empty to clear)")
        .default(current_desc.clone())
        .allow_empty(true)
        .interact_text()?;
    if desc_input != current_desc {
        req.description = Some(desc_input);
    }

    // Period
    let period_input: String = Input::new()
        .with_prompt("Period (e.g., 5m, 1h, 1d)")
        .default(format_duration(check.period_seconds))
        .interact_text()?;
    let new_period = parse_duration(&period_input)?;
    if new_period != check.period_seconds {
        req.period_seconds = Some(new_period);
    }

    // Grace
    let grace_input: String = Input::new()
        .with_prompt("Grace period (e.g., 30s, 5m)")
        .default(format_duration(check.grace_seconds))
        .interact_text()?;
    let new_grace = parse_duration(&grace_input)?;
    if new_grace != check.grace_seconds {
        req.grace_seconds = Some(new_grace);
    }

    // Tags
    let current_tags = check.tags.join(", ");
    let tags_input: String = Input::new()
        .with_prompt("Tags (comma-separated)")
        .default(current_tags)
        .allow_empty(true)
        .interact_text()?;
    let new_tags: Vec<String> = tags_input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if new_tags != check.tags {
        req.tags = Some(new_tags);
    }

    // Alert after failures
    let current_aaf = check.alert_after_failures.unwrap_or(1);
    let aaf_input: String = Input::new()
        .with_prompt("Alert after failures (1-100)")
        .default(current_aaf.to_string())
        .interact_text()?;
    let new_aaf: i32 = aaf_input.parse().unwrap_or(current_aaf);
    if new_aaf != current_aaf {
        req.alert_after_failures = Some(new_aaf);
    }

    // Late after ratio
    let lar_input: String = Input::new()
        .with_prompt("Late after ratio (0.0-1.0)")
        .default(format!("{:.2}", check.late_after_ratio))
        .interact_text()?;
    let new_lar: f32 = lar_input.parse().unwrap_or(check.late_after_ratio);
    if (new_lar - check.late_after_ratio).abs() > f32::EPSILON {
        req.late_after_ratio = Some(new_lar);
    }

    // Max runtime
    let current_max_runtime = check
        .max_runtime_seconds
        .map(format_duration)
        .unwrap_or_default();
    let max_runtime_input: String = Input::new()
        .with_prompt("Max runtime (e.g., 5m, empty for none)")
        .default(current_max_runtime.clone())
        .allow_empty(true)
        .interact_text()?;
    if max_runtime_input != current_max_runtime {
        if max_runtime_input.is_empty() {
            // User wants to clear max_runtime - but we can only set it, not clear
            // Skip for now since the API uses Option
        } else {
            let new_max_runtime = parse_duration(&max_runtime_input)?;
            req.max_runtime_seconds = Some(new_max_runtime);
        }
    }

    // Missed before alert
    let mba_input: String = Input::new()
        .with_prompt("Missed before alert (1-100)")
        .default(check.missed_before_alert.to_string())
        .interact_text()?;
    let new_mba: i32 = mba_input.parse().unwrap_or(check.missed_before_alert);
    if new_mba != check.missed_before_alert {
        req.missed_before_alert = Some(new_mba);
    }

    Ok(req)
}

// ============================================================================
// Helpers
// ============================================================================

/// Validate a slug matches backend requirements
fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 100 {
        return Err(anyhow!("Slug must be 1-100 characters"));
    }
    // Match backend SLUG_REGEX: lowercase letters, numbers, hyphens only
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow!(
            "Slug must contain only lowercase letters, numbers, and hyphens"
        ));
    }
    Ok(())
}

/// Convert a slug to a title-cased display name
fn slug_to_title(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Validate a cron expression for CLI use
fn validate_cron_cli(expr: &str) -> Result<()> {
    // Reject @daily/@hourly shortcuts (not supported, give helpful error)
    if expr.trim().starts_with('@') {
        let suggestion = match expr.trim() {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            _ => "standard 5-field format",
        };
        return Err(anyhow!(
            "Cron shortcut '{}' is not supported. Use {} instead.",
            expr.trim(),
            suggestion
        ));
    }

    validate_cron_expression(expr).map_err(|e| anyhow!(e))
}

/// Validate an IANA timezone string
fn validate_timezone(tz: &str) -> Result<chrono_tz::Tz> {
    tz.parse::<chrono_tz::Tz>().map_err(|_| {
        anyhow!(
            "Invalid timezone '{}'. Use IANA format like 'America/New_York' or 'Asia/Manila'.\n\
             Note: UTC offsets like 'UTC+8' are not supported.",
            tz
        )
    })
}

/// Calculate smart grace period (10% of period, clamped to 5min-1hour)
fn smart_grace(period_seconds: i32) -> i32 {
    let grace = (period_seconds as f64 * 0.1) as i32;
    grace.clamp(300, 3600)
}

/// Style a status string with appropriate color
fn styled_status(status: &str) -> console::StyledObject<&str> {
    use console::style;
    match status {
        "up" | "success" | "healthy" => style(status).green(),
        "down" | "missing" | "fail" | "critical" => style(status).red().bold(),
        "late" | "overrunning" | "warning" | "attention_needed" => style(status).yellow(),
        "running" | "start" => style(status).cyan(),
        _ => style(status).dim(),
    }
}

/// Parse duration string supporting various formats (e.g., "5m", "1h", "5 minutes", "2d")
fn parse_duration(s: &str) -> Result<i32> {
    let s = s.trim().to_lowercase();

    // Try parsing as raw seconds first
    if let Ok(secs) = s.parse::<i32>() {
        return Ok(secs);
    }

    // Try regex-based parsing for flexible formats like "5 minutes", "1h", etc.
    let re = Regex::new(
        r"^(\d+)\s*(s|sec|secs|second|seconds|m|min|mins|minute|minutes|h|hr|hrs|hour|hours|d|day|days)$",
    )
    .unwrap();

    if let Some(caps) = re.captures(&s) {
        let value: i32 = caps[1]
            .parse()
            .map_err(|_| anyhow!("Invalid number in duration: {}", s))?;

        let unit = &caps[2];
        let multiplier = match unit {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            _ => return Err(anyhow!("Unknown unit '{}' in: {}", unit, s)),
        };

        return Ok(value * multiplier);
    }

    Err(anyhow!(
        "Invalid duration '{}'. Use formats like: 30s, 5m, 1h, 2d",
        s
    ))
}

/// Print check creation success in human-readable format
fn print_check_created(ctx: &Context, check: &Check, ping_url: &str, grace_auto: bool) {
    let dashboard_url = format!("{}/checks/{}", ctx.app_url(), check.slug);
    let grace_suffix = if grace_auto { " (auto)" } else { "" };

    println!("Created check: {}\n", check.name);
    println!("  Slug:     {}", check.slug);

    if let Some(cron) = &check.cron_expression {
        let tz = check.timezone.as_deref().unwrap_or("org default");
        println!("  Schedule: {} ({})", cron, tz);
    } else {
        println!("  Period:   {}", format_duration(check.period_seconds));
    }
    println!(
        "  Grace:    {}{}\n",
        format_duration(check.grace_seconds),
        grace_suffix
    );

    println!("  Dashboard: {}", dashboard_url);
    println!("  Ping URL:  {}\n", ping_url);

    println!("  Test with:");
    println!("    curl -fsS {}\n", ping_url);

    println!("  Monitor with:");
    println!("    pakyas monitor {} -- your-command\n", check.slug);

    println!("  CI/CD (no auth required):");
    println!(
        "    pakyas monitor --public-id {} -- your-command",
        check.public_id
    );
}

/// Print check creation success in JSON format
fn print_check_json(ctx: &Context, check: &Check, ping_url: &str, grace_auto: bool) {
    let dashboard_url = format!("{}/checks/{}", ctx.app_url(), check.slug);

    // Compute effective timezone and source
    let (effective_tz, timezone_source) = if check.timezone.is_some() {
        (check.timezone.as_deref().unwrap(), "check")
    } else if let Some(org_tz) = ctx.config.active_org_timezone.as_deref() {
        (org_tz, "org")
    } else {
        ("UTC", "utc_fallback")
    };

    let output = serde_json::json!({
        "id": check.id,
        "public_id": check.public_id,
        "slug": check.slug,
        "name": check.name,
        "ping_url": ping_url,
        "dashboard_url": dashboard_url,
        "schedule_type": if check.cron_expression.is_some() { "cron" } else { "simple" },
        "cron_expression": check.cron_expression,
        "timezone": check.timezone,
        "effective_timezone": effective_tz,
        "timezone_source": timezone_source,
        "period_seconds": check.period_seconds,
        "grace_seconds": check.grace_seconds,
        "grace_auto": grace_auto,
    });
    println!("{}", serde_json::to_string(&output).unwrap());
}

/// Print dry-run output showing what would be created
#[allow(clippy::too_many_arguments)]
fn print_dry_run(
    slug: &str,
    name: &Option<String>,
    cron_expression: &Option<String>,
    period_seconds: i32,
    grace_seconds: i32,
    grace_auto: bool,
    effective_tz: chrono_tz::Tz,
    tz_source: &str,
) {
    let display_name = name.clone().unwrap_or_else(|| slug_to_title(slug));
    let grace_suffix = if grace_auto { " (auto)" } else { "" };

    println!("Would create check:\n");
    println!("  Slug:     {}", slug);
    println!("  Name:     {}", display_name);

    if let Some(cron) = cron_expression {
        println!("  Schedule: {}", cron);

        // Show effective timezone with source
        let tz_label = match tz_source {
            "check" => format!("{} (from --tz)", effective_tz),
            "org" => format!("{} (org default)", effective_tz),
            _ => format!("{} (fallback)", effective_tz),
        };
        println!("  Timezone: {}", tz_label);

        // Show next runs in effective timezone
        let next_runs = next_cron_times_in_tz(cron, effective_tz, 3);
        if !next_runs.is_empty() {
            println!("\n  Next runs:");
            for dt in next_runs {
                println!("    {}", dt.format("%Y-%m-%d %H:%M %Z"));
            }
        }
    } else {
        println!("  Period:   {}", format_duration(period_seconds));
    }

    println!(
        "  Grace:    {}{}",
        format_duration(grace_seconds),
        grace_suffix
    );
}

/// Resolve a check by slug or ID, using cache when possible
async fn resolve_check(ctx: &Context, project_id: &str, slug_or_id: &str) -> Result<Check> {
    let client = ApiClient::new(ctx)?;

    // Try to parse as UUID first
    if let Ok(uuid) = Uuid::parse_str(slug_or_id) {
        let url = format!("/api/v1/checks/{}", uuid);
        return client.get(&url).await;
    }

    // Try cache lookup
    let cache = CheckCache::load()?;
    if let Some(entry) = cache.get(project_id, slug_or_id) {
        let url = format!("/api/v1/checks/{}", entry.check_id);
        match client.get::<Check>(&url).await {
            Ok(check) => return Ok(check),
            Err(_) => {
                // Cache miss (check deleted?), invalidate and continue
                let mut cache = CheckCache::load()?;
                cache.invalidate(project_id, slug_or_id);
                cache.save()?;
            }
        }
    }

    // Fetch all checks and find by slug
    let url = format!("/api/v1/checks?project_id={}", project_id);
    let checks: Vec<Check> = client.get(&url).await?;

    // Update cache
    let mut cache = CheckCache::load()?;
    cache.update_from_checks(project_id, checks.iter().cloned());
    cache.save()?;

    checks
        .into_iter()
        .find(|c| c.slug.eq_ignore_ascii_case(slug_or_id))
        .ok_or_else(|| CliError::CheckNotFound(slug_or_id.to_string()).into())
}

/// Resolve a check by slug or ID using org context (for read-only commands)
/// This allows finding checks across all projects in the organization.
async fn resolve_check_by_org(ctx: &Context, org_id: &str, slug_or_id: &str) -> Result<Check> {
    let client = ApiClient::new(ctx)?;

    // Try to parse as UUID first
    if let Ok(uuid) = Uuid::parse_str(slug_or_id) {
        let url = format!("/api/v1/checks/{}", uuid);
        return client.get(&url).await;
    }

    // Try cache lookup (using org_id as key)
    let cache = CheckCache::load()?;
    if let Some(entry) = cache.get(org_id, slug_or_id) {
        let url = format!("/api/v1/checks/{}", entry.check_id);
        match client.get::<Check>(&url).await {
            Ok(check) => return Ok(check),
            Err(_) => {
                // Cache miss (check deleted?), invalidate and continue
                let mut cache = CheckCache::load()?;
                cache.invalidate(org_id, slug_or_id);
                cache.save()?;
            }
        }
    }

    // Fetch all checks from org and find by slug
    let url = format!("/api/v1/checks?org_id={}", org_id);
    let checks: Vec<CheckWithProject> = client.get(&url).await?;

    // Update cache with org_id
    let mut cache = CheckCache::load()?;
    cache.update_from_checks(org_id, checks.iter().map(|c| c.check.clone()));
    cache.save()?;

    checks
        .into_iter()
        .map(|c| c.check)
        .find(|c| c.slug.eq_ignore_ascii_case(slug_or_id))
        .ok_or_else(|| CliError::CheckNotFound(slug_or_id.to_string()).into())
}

/// Resolve a check's public_id by slug (for ping commands)
pub async fn resolve_public_id(ctx: &Context, project_id: &str, slug: &str) -> Result<Uuid> {
    // Try cache first
    let cache = CheckCache::load()?;
    if let Some(entry) = cache.get(project_id, slug) {
        return Ok(entry.public_id);
    }

    // Fetch from API
    let check = resolve_check(ctx, project_id, slug).await?;
    Ok(check.public_id)
}

/// Format seconds as human-readable duration
fn format_duration(seconds: i32) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        let hours = seconds / 3600;
        let mins = (seconds % 3600) / 60;
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    } else {
        let days = seconds / 86400;
        let hours = (seconds % 86400) / 3600;
        if hours > 0 {
            format!("{}d {}h", days, hours)
        } else {
            format!("{}d", days)
        }
    }
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

/// Format ping type with color
fn format_ping_type(ping_type: &str) -> String {
    match ping_type.to_lowercase().as_str() {
        "success" => format_status("up"),
        "fail" => format_status("down"),
        "start" => format_status("running"),
        _ => ping_type.to_string(),
    }
}

// ============================================================================
// Inspect Command
// ============================================================================

/// Response types for inspect endpoint
#[derive(Debug, Deserialize, Serialize)]
struct InspectResponse {
    check_id: Uuid,
    public_id: Uuid,
    name: String,
    slug: String,
    ping_url: String,
    project_name: String,
    status: String,
    status_since: DateTime<Utc>,
    is_critical: bool,
    critical_reason: Option<String>,
    schedule: ScheduleInfo,
    last_signal: Option<LastSignalInfo>,
    alerting: AlertingInfo,
    maintenance: MaintenanceInfo,
    stats: StatsInfo,
}

#[derive(Debug, Deserialize, Serialize)]
struct ScheduleInfo {
    kind: String,
    period_seconds: i32,
    cron_expression: Option<String>,
    grace_seconds: i32,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LastSignalInfo {
    signal_type: String,
    at: DateTime<Utc>,
    duration_ms: Option<i32>,
    source_ip: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AlertingInfo {
    enabled: bool,
    alert_after_miss_pings: i32,
    alert_after_fail_pings: i32,
    consecutive_missed_pings: i32,
    consecutive_fail_pings: i32,
    recipient_count: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct MaintenanceInfo {
    in_maintenance: bool,
    reason: Option<String>,
    ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StatsInfo {
    success_rate_24h: Option<f64>,
    total_pings_24h: i64,
    p95_duration_ms: Option<i64>,
}

/// Inspect a check's current state and configuration
async fn inspect(ctx: &Context, slug_or_id: &str, verbose: bool) -> Result<()> {
    use crate::cli::OutputFormat;
    use crate::output::OutputConfig;
    use console::style;

    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    if verbose {
        eprintln!("[verbose] Fetching inspect data for check: {}", check.id);
    }

    let url = format!("/api/v1/checks/{}/inspect", check.id);
    let response: InspectResponse = client.get(&url).await?;

    // JSON output
    if matches!(ctx.output_format(), OutputFormat::Json) {
        print_single(ctx, &response)?;
        return Ok(());
    }

    let config = OutputConfig::from_context(ctx);

    // Human-readable output
    println!(
        "{}  {}  \"{}\"",
        style("CHECK").bold(),
        &response.check_id.to_string()[..8],
        response.name
    );
    println!("{}    {}", style("URL").dim(), response.ping_url);
    println!();

    // State section
    println!("{}", style("STATE").bold().underlined());
    println!("  now          {}", styled_status(&response.status));
    println!(
        "  since        {}",
        crate::output::format_timestamp(response.status_since, &config)
    );
    if response.is_critical {
        let reason = response.critical_reason.as_deref().unwrap_or("unknown");
        println!("  critical     {} ({})", style("yes").red().bold(), reason);
    }
    println!();

    // Schedule section
    println!("{}", style("SCHEDULE").bold().underlined());
    println!("  kind         {}", response.schedule.kind);
    if let Some(cron) = &response.schedule.cron_expression {
        println!("  cron         {}", cron);
        if let Some(tz) = &response.schedule.timezone {
            println!("  timezone     {}", tz);
        }
    } else {
        println!(
            "  every        {}",
            format_duration(response.schedule.period_seconds)
        );
    }
    println!(
        "  grace        {}",
        format_duration(response.schedule.grace_seconds)
    );
    println!();

    // Last signal section
    println!("{}", style("LAST SIGNAL").bold().underlined());
    if let Some(signal) = &response.last_signal {
        let signal_styled = match signal.signal_type.as_str() {
            "success" => style(&signal.signal_type).green(),
            "fail" => style(&signal.signal_type).red(),
            "start" => style(&signal.signal_type).cyan(),
            _ => style(&signal.signal_type),
        };
        println!(
            "  at           {}",
            crate::output::format_timestamp(signal.at, &config)
        );
        println!("  status       {}", signal_styled);
        if let Some(duration) = signal.duration_ms {
            println!("  latency      {}ms", duration);
        }
        if let Some(ip) = &signal.source_ip {
            println!("  source       {}", ip);
        }
    } else {
        println!("  {}", style("(no signals received)").dim());
    }
    println!();

    // Alerting section
    println!("{}", style("ALERTING").bold().underlined());
    let enabled_str = if response.alerting.enabled {
        style("yes").green().to_string()
    } else {
        style("no").red().to_string()
    };
    println!("  enabled      {}", enabled_str);
    if response.alerting.enabled {
        println!(
            "  thresholds   {} misses, {} fails before alert",
            response.alerting.alert_after_miss_pings, response.alerting.alert_after_fail_pings
        );
        println!(
            "  consecutive  {} misses, {} fails",
            response.alerting.consecutive_missed_pings, response.alerting.consecutive_fail_pings
        );
        println!("  recipients   {}", response.alerting.recipient_count);
    }
    println!();

    // Maintenance section
    if response.maintenance.in_maintenance {
        println!("{}", style("MAINTENANCE").bold().underlined());
        println!("  active       {}", style("yes").yellow());
        if let Some(reason) = &response.maintenance.reason {
            println!("  reason       {}", reason);
        }
        if let Some(ends) = response.maintenance.ends_at {
            println!(
                "  ends         {}",
                crate::output::format_timestamp(ends, &config)
            );
        }
        println!();
    }

    // Stats section
    println!("{}", style("STATS (24h)").bold().underlined());
    if let Some(rate) = response.stats.success_rate_24h {
        println!("  success      {:.1}%", rate * 100.0);
    }
    println!("  pings        {}", response.stats.total_pings_24h);
    if let Some(p95) = response.stats.p95_duration_ms {
        println!("  p95 latency  {}ms", p95);
    }

    Ok(())
}

// ============================================================================
// Doctor Command
// ============================================================================

/// Response types for doctor endpoint
#[derive(Debug, Deserialize, Serialize)]
struct DoctorReport {
    check_id: Uuid,
    check_name: String,
    analyzed_at: DateTime<Utc>,
    deep: bool,
    findings: Vec<DoctorFinding>,
    status: String,
    summary: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DoctorFinding {
    code: String,
    severity: String,
    title: String,
    description: String,
    #[serde(default)]
    details: Option<serde_json::Value>,
    #[serde(default)]
    suggested_actions: Vec<String>,
}

/// Run diagnostic analysis on a check
async fn doctor(
    ctx: &Context,
    slug_or_id: &str,
    deep: bool,
    fail_on: FailOnSeverity,
    verbose: bool,
) -> Result<()> {
    use crate::cli::OutputFormat;
    use console::style;

    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    if verbose {
        eprintln!("[verbose] Running doctor analysis for check: {}", check.id);
        if deep {
            eprintln!("[verbose] Deep analysis enabled");
        }
    }

    let url = format!("/api/v1/checks/{}/doctor?deep={}", check.id, deep);
    let report: DoctorReport = client.get(&url).await?;

    // JSON output
    if matches!(ctx.output_format(), OutputFormat::Json) {
        print_single(ctx, &report)?;
        // Check exit code based on findings
        return check_doctor_exit(report, fail_on);
    }

    // Human-readable output
    println!(
        "{}  {}  \"{}\"",
        style("DOCTOR").bold(),
        &report.check_id.to_string()[..8],
        report.check_name
    );

    // Status line
    let status_styled = match report.status.as_str() {
        "healthy" => style("✓ Healthy").green().to_string(),
        "attention_needed" => style("! Attention needed").yellow().to_string(),
        "critical" => style("✗ Critical").red().bold().to_string(),
        _ => report.status.clone(),
    };
    println!("Result: {}", status_styled);
    println!();

    if report.findings.is_empty() {
        println!("{}", style("No issues found").green());
        return Ok(());
    }

    // Findings
    println!("{}", style("DIAGNOSIS").bold().underlined());
    for (i, finding) in report.findings.iter().enumerate() {
        let severity_symbol = match finding.severity.as_str() {
            "error" => style("✗").red().bold(),
            "warning" => style("!").yellow(),
            "info" => style("ℹ").blue(),
            _ => style("•"),
        };

        println!("  {}) {} {}", i + 1, severity_symbol, finding.title);
        if !finding.description.is_empty() {
            // Wrap description at 60 chars and indent
            for line in textwrap::wrap(&finding.description, 55) {
                println!("     {}", style(&*line).dim());
            }
        }

        // Show details if present
        if let Some(details) = &finding.details {
            println!("     {}", style("Evidence:").dim());
            if let Some(obj) = details.as_object() {
                for (key, value) in obj {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => serde_json::to_string(value).unwrap_or_default(),
                    };
                    println!("       - {}: {}", key, value_str);
                }
            }
        }

        // Show suggested actions
        if !finding.suggested_actions.is_empty() {
            println!("     {}", style("Next actions:").dim());
            for action in &finding.suggested_actions {
                println!("       - {}", action);
            }
        }
        println!();
    }

    check_doctor_exit(report, fail_on)
}

/// Check if doctor should exit with error based on findings and fail_on setting
fn check_doctor_exit(report: DoctorReport, fail_on: FailOnSeverity) -> Result<()> {
    let has_error = report.findings.iter().any(|f| f.severity == "error");
    let has_warning = report.findings.iter().any(|f| f.severity == "warning");
    let has_info = report.findings.iter().any(|f| f.severity == "info");

    let should_fail = match fail_on {
        FailOnSeverity::Error => has_error,
        FailOnSeverity::Warning => has_error || has_warning,
        FailOnSeverity::Info => has_error || has_warning || has_info,
    };

    if should_fail {
        std::process::exit(exit_codes::ISSUES);
    }

    Ok(())
}

// ============================================================================
// Tail Command
// ============================================================================

/// Response types for events endpoint
#[derive(Debug, Deserialize, Serialize)]
struct EventsResponse {
    events: Vec<EventItem>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EventItem {
    id: i64,
    event_type: String,
    occurred_at: DateTime<Utc>,
    #[serde(default)]
    effective_at: Option<DateTime<Utc>>,
    source: String,
    payload: serde_json::Value,
    from_status: Option<String>,
    to_status: Option<String>,
    summary: String,
}

/// Stream timeline events for a check
async fn tail(
    ctx: &Context,
    slug_or_id: &str,
    since: &str,
    types: Option<&str>,
    follow: bool,
    limit: i64,
    verbose: bool,
) -> Result<()> {
    use crate::cli::OutputFormat;
    use crate::output::{OutputConfig, print_ndjson};
    use console::style;
    use std::collections::HashSet;

    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;
    let client = ApiClient::new(ctx)?;

    let config = OutputConfig::from_context(ctx);
    let is_ndjson = matches!(ctx.output_format(), OutputFormat::Ndjson);
    let is_json = matches!(ctx.output_format(), OutputFormat::Json);

    if verbose {
        eprintln!("[verbose] Fetching events for check: {}", check.id);
        eprintln!("[verbose] Since: {}, Follow: {}", since, follow);
    }

    // Build base URL with query params
    let mut base_url = format!("/api/v1/checks/{}/events?limit={}", check.id, limit);
    if !since.is_empty() {
        base_url.push_str(&format!("&since={}", since));
    }
    if let Some(t) = types {
        base_url.push_str(&format!("&types={}", t));
    }

    // Track seen event IDs to avoid duplicates in follow mode
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut first_batch = true;
    let mut all_events: Vec<EventItem> = Vec::new();

    // Print header for human output
    if !is_ndjson && !is_json && !follow {
        println!(
            "{}  \"{}\"  since={}",
            style("TAIL").bold(),
            check.name,
            since
        );
        println!();
    }

    loop {
        // Build URL with cursor if present
        let url = if let Some(ref c) = cursor {
            format!("{}&cursor={}", base_url, c)
        } else {
            base_url.clone()
        };

        let response: EventsResponse = client.get(&url).await?;

        // Filter out already-seen events
        let new_events: Vec<EventItem> = response
            .events
            .into_iter()
            .filter(|e| seen_ids.insert(e.id))
            .collect();

        if !new_events.is_empty() {
            if is_ndjson {
                // Stream each event as NDJSON
                for event in &new_events {
                    print_ndjson(event)?;
                }
            } else if is_json {
                // Collect for final JSON output (non-follow mode)
                all_events.extend(new_events.clone());
            } else {
                // Human-readable output
                for event in &new_events {
                    print_event_line(event, &config);
                }
            }
        }

        // Update cursor for next iteration
        cursor = response.next_cursor;

        // If not following, exit after first complete fetch
        if !follow {
            // For JSON mode, output all collected events
            if is_json {
                print_single(ctx, &all_events)?;
            }
            break;
        }

        // In follow mode, if no more results, wait and poll again
        if !response.has_more {
            if first_batch && new_events.is_empty() && !is_ndjson && !is_json {
                println!("{}", style("(no events in time range, waiting...)").dim());
            }
            first_batch = false;

            // Exponential backoff: start at 2s, max 30s
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Reset cursor to fetch new events from beginning
            cursor = None;
        }
    }

    Ok(())
}

/// Print a single event line in human-readable format
fn print_event_line(event: &EventItem, config: &crate::output::OutputConfig) {
    use console::style;

    // Format timestamp as HH:MM:SS
    let time_str = event.occurred_at.format("%H:%M:%S").to_string();

    // Determine symbol and color based on event type
    let (symbol, event_label) = match event.event_type.as_str() {
        "run_started" => (style("▶").cyan(), "signal"),
        "run_finished" => {
            // Check if success or fail from summary
            if event.summary.contains("success") {
                (style("✓").green(), "signal")
            } else {
                (style("✗").red(), "signal")
            }
        }
        "status_changed" => {
            // Check if going to bad state
            if let Some(to) = &event.to_status {
                match to.as_str() {
                    "down" | "missing" => (style("!").red().bold(), "state"),
                    "late" | "overrunning" => (style("!").yellow(), "state"),
                    "up" => (style("✓").green(), "state"),
                    _ => (style("•").dim(), "state"),
                }
            } else {
                (style("•").dim(), "state")
            }
        }
        "alert_decision" => {
            if event.summary.contains("sent") || event.summary.contains("fired") {
                (style("⚠").red().bold(), "alert")
            } else {
                (style("○").dim(), "alert")
            }
        }
        _ => (style("•").dim(), &event.event_type[..]),
    };

    // Print formatted line
    let symbol_str = if config.plain {
        match event.event_type.as_str() {
            "run_started" => "[START]",
            "run_finished" => {
                if event.summary.contains("success") {
                    "[OK]"
                } else {
                    "[FAIL]"
                }
            }
            "status_changed" => "[STATE]",
            "alert_decision" => "[ALERT]",
            _ => "[-]",
        }
    } else {
        &symbol.to_string()
    };

    println!(
        "{}  {} {:8} {}",
        style(&time_str).dim(),
        symbol_str,
        event_label,
        event.summary
    );
}
