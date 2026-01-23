//! Check creation workflow: create and create_interactive.

use crate::cache::CheckCache;
use crate::client::ApiClient;
use crate::config::Context;
use crate::cron::effective_period_from_cron;
use crate::error::CliError;
use anyhow::{Result, anyhow};
use dialoguer::{Input, Select};
use uuid::Uuid;

use super::helpers::{
    format_duration, parse_duration, print_check_created, print_check_json, print_dry_run,
    slug_to_title, smart_grace, validate_cron_cli, validate_slug, validate_timezone,
};
use super::types::{Check, CreateCheckRequest};

/// Create a new check
#[allow(clippy::too_many_arguments)]
pub async fn create(
    ctx: &Context,
    slug: String,
    name: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    every: Option<String>,
    missing_after: Option<String>,
    description: Option<String>,
    tags: Option<String>,
    alert_after_miss_pings: Option<i32>,
    alert_after_fail_pings: Option<i32>,
    max_runtime: Option<String>,
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
    let (cron_expression, timezone, period_seconds, missing_after_seconds, grace_auto) =
        if let Some(cron_expr) = &cron {
            validate_cron_cli(cron_expr)?;

            let period = effective_period_from_cron(cron_expr).unwrap_or(3600);

            let (grace_val, auto) = if let Some(g) = &missing_after {
                (parse_duration(g)?, false)
            } else {
                (smart_grace(period), true)
            };

            (Some(cron_expr.clone()), tz.clone(), period, grace_val, auto)
        } else {
            let every_str = every.as_ref().unwrap();
            let period = parse_duration(every_str)?;

            let (grace_val, auto) = if let Some(g) = &missing_after {
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
            missing_after_seconds,
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

    // Parse optional fields
    let tags_vec = tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let max_runtime_seconds = max_runtime.map(|m| parse_duration(&m)).transpose()?;

    let req = CreateCheckRequest {
        project_id: project_uuid,
        name: display_name.clone(),
        slug: slug.clone(),
        period_seconds,
        missing_after_seconds,
        description,
        cron_expression,
        timezone,
        tags: tags_vec,
        alert_after_miss_pings,
        alert_after_fail_pings,
        max_runtime_seconds,
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
pub async fn create_interactive(
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

    let (missing_after_seconds, grace_auto) = if grace_input.trim().is_empty() {
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

    // Create (interactive mode doesn't prompt for advanced fields - use defaults)
    let req = CreateCheckRequest {
        project_id: project_uuid,
        name: display_name,
        slug: slug.clone(),
        period_seconds,
        missing_after_seconds,
        description: final_description,
        cron_expression,
        timezone,
        tags: None,
        alert_after_miss_pings: None,
        alert_after_fail_pings: None,
        max_runtime_seconds: None,
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
