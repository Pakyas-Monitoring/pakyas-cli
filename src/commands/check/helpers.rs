//! Helper functions for validation, formatting, and resolution.

use crate::cache::CheckCache;
use crate::client::ApiClient;
use crate::config::Context;
use crate::cron::{next_cron_times_in_tz, validate_cron_expression};
use crate::error::CliError;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use regex::Regex;
use uuid::Uuid;

use super::types::{Check, CheckWithProject};

// ============================================================================
// Validation
// ============================================================================

/// Validate a slug matches backend requirements
pub fn validate_slug(slug: &str) -> Result<()> {
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

/// Validate a cron expression for CLI use
pub fn validate_cron_cli(expr: &str) -> Result<()> {
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
pub fn validate_timezone(tz: &str) -> Result<chrono_tz::Tz> {
    tz.parse::<chrono_tz::Tz>().map_err(|_| {
        anyhow!(
            "Invalid timezone '{}'. Use IANA format like 'America/New_York' or 'Asia/Manila'.\n\
             Note: UTC offsets like 'UTC+8' are not supported.",
            tz
        )
    })
}

// ============================================================================
// Parsing & Formatting
// ============================================================================

/// Parse duration string supporting various formats (e.g., "5m", "1h", "5 minutes", "2d")
pub fn parse_duration(s: &str) -> Result<i32> {
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

/// Format seconds as human-readable duration
pub fn format_duration(seconds: i32) -> String {
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
pub fn format_relative_time(dt: Option<DateTime<Utc>>) -> String {
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
pub fn format_ping_type(ping_type: &str) -> String {
    use crate::output::format_status;
    match ping_type.to_lowercase().as_str() {
        "success" => format_status("up"),
        "fail" => format_status("down"),
        "start" => format_status("running"),
        _ => ping_type.to_string(),
    }
}

/// Style a status string with appropriate color
pub fn styled_status(status: &str) -> console::StyledObject<&str> {
    use console::style;
    match status {
        "up" | "success" | "healthy" => style(status).green(),
        "down" | "missing" | "fail" | "critical" => style(status).red().bold(),
        "late" | "overrunning" | "warning" | "attention_needed" => style(status).yellow(),
        "running" | "start" => style(status).cyan(),
        _ => style(status).dim(),
    }
}

/// Convert a slug to a title-cased display name
pub fn slug_to_title(slug: &str) -> String {
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

/// Calculate smart grace period (10% of period, clamped to 5min-1hour)
pub fn smart_grace(period_seconds: i32) -> i32 {
    let grace = (period_seconds as f64 * 0.1) as i32;
    grace.clamp(300, 3600)
}

// ============================================================================
// Resolution
// ============================================================================

/// Resolve a check by slug or ID, using cache when possible
pub async fn resolve_check(ctx: &Context, project_id: &str, slug_or_id: &str) -> Result<Check> {
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
pub async fn resolve_check_by_org(ctx: &Context, org_id: &str, slug_or_id: &str) -> Result<Check> {
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

// ============================================================================
// Output Helpers
// ============================================================================

/// Print check creation success in human-readable format
pub fn print_check_created(ctx: &Context, check: &Check, ping_url: &str, grace_auto: bool) {
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
pub fn print_check_json(ctx: &Context, check: &Check, ping_url: &str, grace_auto: bool) {
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
pub fn print_dry_run(
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
