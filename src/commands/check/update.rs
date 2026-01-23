//! Check update workflow: update and request builders.

use crate::client::ApiClient;
use crate::config::Context;
use crate::output::{print_success, print_warning};
use anyhow::Result;
use dialoguer::{Confirm, Input};

use super::helpers::{
    format_duration, parse_duration, resolve_check_by_org, validate_cron_cli, validate_timezone,
};
use super::types::{Check, UpdateCheckRequest};

/// Update a check's configuration
#[allow(clippy::too_many_arguments)]
pub async fn update(
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
    print_changes(&check, &req);

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

/// Print the changes that will be applied
fn print_changes(check: &Check, req: &UpdateCheckRequest) {
    println!("\nChanges to '{}':", check.name);

    if let Some(ref new_name) = req.name {
        println!("  Name: {} -> {}", check.name, new_name);
    }
    if let Some(ref new_desc) = req.description {
        let old_desc = check.description.as_deref().unwrap_or("(none)");
        let new_desc_display = if new_desc.is_empty() {
            "(cleared)"
        } else {
            new_desc
        };
        println!("  Description: {} -> {}", old_desc, new_desc_display);
    }
    if let Some(ref new_cron) = req.cron_expression {
        let old_cron = check.cron_expression.as_deref().unwrap_or("(none)");
        let new_cron_display = if new_cron.is_empty() {
            "(cleared - switching to interval)"
        } else {
            new_cron
        };
        println!("  Cron: {} -> {}", old_cron, new_cron_display);
    }
    if let Some(ref new_tz) = req.timezone {
        let old_tz = check.timezone.as_deref().unwrap_or("(org default)");
        let new_tz_display = if new_tz.is_empty() {
            "(org default)"
        } else {
            new_tz
        };
        println!("  Timezone: {} -> {}", old_tz, new_tz_display);
    }
    if let Some(new_period) = req.period_seconds {
        println!(
            "  Period: {} -> {}",
            format_duration(check.period_seconds),
            format_duration(new_period)
        );
    }
    if let Some(new_grace) = req.grace_seconds {
        println!(
            "  Grace: {} -> {}",
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
        println!("  Tags: {} -> {}", old_tags, new_tags_display);
    }
    if let Some(new_aaf) = req.alert_after_failures {
        println!(
            "  Alert after failures: {} -> {}",
            check
                .alert_after_failures
                .map(|v| v.to_string())
                .unwrap_or_else(|| "inherited".to_string()),
            new_aaf
        );
    }
    if let Some(new_lar) = req.late_after_ratio {
        println!(
            "  Late after ratio: {:.0}% -> {:.0}%",
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
            "  Max runtime: {} -> {}",
            old_max,
            format_duration(new_max_runtime)
        );
    }
    if let Some(new_mba) = req.missed_before_alert {
        println!(
            "  Missed before alert: {} -> {}",
            check.missed_before_alert, new_mba
        );
    }
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
