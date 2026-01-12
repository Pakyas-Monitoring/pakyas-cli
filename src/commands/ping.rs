use crate::cli::PingArgs;
use crate::commands::check::resolve_public_id;
use crate::config::Context;
use crate::external_monitors::ExternalMonitorConfig;
use crate::external_ping::{EventType, PingEvent, dispatch_external_pings};
use crate::output::{print_error, print_success};
use crate::ua::user_agent;
use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
use uuid::Uuid;

const PING_TIMEOUT_SECS: u64 = 10;

/// Execute the ping command
pub async fn execute(ctx: &Context, args: PingArgs, verbose: bool) -> Result<()> {
    let project_id = ctx.require_project()?;

    if verbose {
        eprintln!("[verbose] Ping URL base: {}", ctx.ping_url());
        eprintln!("[verbose] Project ID: {}", project_id);
    }

    // Resolve slug to public_id
    let public_id = resolve_public_id(ctx, project_id, &args.slug).await?;

    if verbose {
        eprintln!("[verbose] Resolved public_id: {}", public_id);
    }

    // Build the ping URL with modifier
    let modifier = build_modifier(&args);
    let url = build_ping_url(ctx, public_id, &modifier);

    if verbose {
        eprintln!("[verbose] Sending ping to: {}", url);
    }

    // Send the ping (with optional run_id for START/END pairing and duration for accuracy)
    send_ping(&url, args.run.as_deref(), args.duration_ms).await?;

    // Print appropriate success message
    print_ping_success(&args);

    // Dispatch to external monitors and await completion
    if !args.no_external {
        dispatch_external_ping(&args, verbose).await;
    }

    Ok(())
}

/// Dispatch ping to external monitors and await completion
async fn dispatch_external_ping(args: &PingArgs, verbose: bool) {
    // Show config paths being checked
    if verbose {
        eprintln!("[verbose] Checking external monitors config paths:");
        for path in ExternalMonitorConfig::config_paths() {
            let exists = path.exists();
            eprintln!("[verbose]   {} (exists: {})", path.display(), exists);
        }
    }

    // Load external monitor config
    let external_config = match ExternalMonitorConfig::load() {
        Ok(c) => {
            if verbose {
                if let Ok(path) = ExternalMonitorConfig::path() {
                    eprintln!("[verbose] Using config: {}", path.display());
                }
            }
            c
        }
        Err(e) => {
            if verbose {
                eprintln!("[verbose] Failed to load external monitor config: {}", e);
            }
            return;
        }
    };

    // Build targets for this check
    let monitors = external_config.build_monitors_for_check(&args.slug);

    if verbose {
        eprintln!(
            "[verbose] Loaded {} external monitor(s) for '{}'",
            monitors.len(),
            args.slug
        );
    }

    if monitors.is_empty() {
        return;
    }

    // Build the event based on ping type
    let event = build_external_event(args);

    // Dispatch and await completion
    if let Some(handle) =
        dispatch_external_pings(monitors, event, args.external_timeout_ms, verbose)
    {
        let timeout = Duration::from_millis(args.external_timeout_ms);
        if tokio::time::timeout(timeout, handle).await.is_err() {
            eprintln!("Warning: external ping timed out");
        }
    }
}

/// Build external ping event from args
fn build_external_event(args: &PingArgs) -> PingEvent {
    if args.start {
        PingEvent::start(&args.slug)
    } else if args.fail {
        PingEvent {
            check_slug: args.slug.clone(),
            event_type: EventType::Fail,
            exit_code: Some(1),
            duration_ms: None,
            timestamp: chrono::Utc::now(),
            host: hostname::get().ok().and_then(|h| h.into_string().ok()),
            output: None,
        }
    } else if let Some(exit_code) = args.exit_code {
        PingEvent::completion(&args.slug, exit_code, 0, "")
    } else {
        PingEvent::success(&args.slug, 0)
    }
}

/// Build the URL modifier based on ping arguments
fn build_modifier(args: &PingArgs) -> String {
    if args.start {
        "/start".to_string()
    } else if args.fail {
        "/fail".to_string()
    } else if let Some(exit_code) = args.exit_code {
        format!("/{}", exit_code)
    } else {
        String::new()
    }
}

/// Build the full ping URL
fn build_ping_url(ctx: &Context, public_id: Uuid, modifier: &str) -> String {
    let base = ctx.ping_url();
    format!("{}/{}{}", base.trim_end_matches('/'), public_id, modifier)
}

/// Send a ping to the given URL (GET, no body)
async fn send_ping(url: &str, run_id: Option<&str>, duration_ms: Option<u64>) -> Result<()> {
    send_ping_inner(url, None, run_id, duration_ms).await
}

/// Send a ping with optional body (POST if body present, GET otherwise)
async fn send_ping_inner(
    url: &str,
    body: Option<&str>,
    run_id: Option<&str>,
    duration_ms: Option<u64>,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
        .build()?;

    let response = match body {
        Some(b) => {
            let mut request = client
                .post(url)
                .header(reqwest::header::USER_AGENT, user_agent())
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(b.to_owned());

            // Add run_id header for START/END pairing
            if let Some(rid) = run_id {
                request = request.header("X-Pakyas-Run", rid);
            }

            // Add duration header for accurate timing
            if let Some(duration) = duration_ms {
                request = request.header("X-Pakyas-Duration", duration.to_string());
            }

            request.send().await?
        }
        None => {
            let mut request = client
                .get(url)
                .header(reqwest::header::USER_AGENT, user_agent());

            // Add run_id header for START/END pairing
            if let Some(rid) = run_id {
                request = request.header("X-Pakyas-Run", rid);
            }

            // Add duration header for accurate timing
            if let Some(duration) = duration_ms {
                request = request.header("X-Pakyas-Duration", duration.to_string());
            }

            request.send().await?
        }
    };

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Ping failed with status {}: {}", status, body);
    }
}

/// Print success message based on ping type
fn print_ping_success(args: &PingArgs) {
    if args.start {
        print_success(&format!("Sent start ping for '{}'", args.slug));
    } else if args.fail {
        print_success(&format!("Sent fail ping for '{}'", args.slug));
    } else if let Some(exit_code) = args.exit_code {
        if exit_code == 0 {
            print_success(&format!(
                "Sent success ping for '{}' (exit code 0)",
                args.slug
            ));
        } else {
            print_success(&format!(
                "Sent fail ping for '{}' (exit code {})",
                args.slug, exit_code
            ));
        }
    } else {
        print_success(&format!("Sent success ping for '{}'", args.slug));
    }
}

/// Send a ping directly with public_id (used by monitor command)
pub async fn send_ping_direct(ping_url: &str, public_id: Uuid, modifier: &str) -> Result<()> {
    send_ping_direct_with_body(ping_url, public_id, modifier, None).await
}

/// Send a ping directly with public_id and optional body (used by monitor command for failures)
pub async fn send_ping_direct_with_body(
    ping_url: &str,
    public_id: Uuid,
    modifier: &str,
    body: Option<&str>,
) -> Result<()> {
    let url = format!(
        "{}/{}{}",
        ping_url.trim_end_matches('/'),
        public_id,
        modifier
    );

    // Fire-and-forget: don't fail the wrapper command if ping fails
    // Note: No run_id or duration here - monitor command uses its own internal functions
    match send_ping_inner(&url, body, None, None).await {
        Ok(_) => Ok(()),
        Err(e) => {
            print_error(&format!("Warning: ping failed: {}", e));
            Ok(()) // Don't propagate error
        }
    }
}
