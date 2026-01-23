//! Inspect command for detailed check state and configuration.

use crate::cli::OutputFormat;
use crate::client::ApiClient;
use crate::config::Context;
use crate::output::{OutputConfig, print_single};
use anyhow::Result;
use chrono::{DateTime, Utc};
use console::style;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::helpers::{format_duration, resolve_check_by_org, styled_status};

// ============================================================================
// Response Types
// ============================================================================

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

// ============================================================================
// Command Implementation
// ============================================================================

/// Inspect a check's current state and configuration
pub async fn inspect(ctx: &Context, slug_or_id: &str, verbose: bool) -> Result<()> {
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
