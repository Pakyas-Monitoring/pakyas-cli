//! Tail command for streaming timeline events.

use crate::cli::OutputFormat;
use crate::client::ApiClient;
use crate::config::Context;
use crate::output::{OutputConfig, print_ndjson, print_single};
use anyhow::Result;
use chrono::{DateTime, Utc};
use console::style;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::helpers::resolve_check_by_org;

// ============================================================================
// Response Types
// ============================================================================

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

// ============================================================================
// Command Implementation
// ============================================================================

/// Stream timeline events for a check
pub async fn tail(
    ctx: &Context,
    slug_or_id: &str,
    since: &str,
    types: Option<&str>,
    follow: bool,
    limit: i64,
    verbose: bool,
) -> Result<()> {
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
fn print_event_line(event: &EventItem, config: &OutputConfig) {
    // Format timestamp as HH:MM:SS
    let time_str = event.occurred_at.format("%H:%M:%S").to_string();

    // Determine symbol and color based on event type
    let (symbol, event_label) = match event.event_type.as_str() {
        "run_started" => (style(">").cyan(), "signal"),
        "run_finished" => {
            // Check if success or fail from summary
            if event.summary.contains("success") {
                (style("V").green(), "signal")
            } else {
                (style("X").red(), "signal")
            }
        }
        "status_changed" => {
            // Check if going to bad state
            if let Some(to) = &event.to_status {
                match to.as_str() {
                    "down" | "missing" => (style("!").red().bold(), "state"),
                    "late" | "overrunning" => (style("!").yellow(), "state"),
                    "up" => (style("V").green(), "state"),
                    _ => (style("*").dim(), "state"),
                }
            } else {
                (style("*").dim(), "state")
            }
        }
        "alert_decision" => {
            if event.summary.contains("sent") || event.summary.contains("fired") {
                (style("W").red().bold(), "alert")
            } else {
                (style("o").dim(), "alert")
            }
        }
        _ => (style("*").dim(), &event.event_type[..]),
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
