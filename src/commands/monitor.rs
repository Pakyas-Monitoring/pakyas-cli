use crate::cli::MonitorArgs;
use crate::commands::check::resolve_public_id;
use crate::config::Context;
use crate::error::CliError;
use crate::external_monitors::ExternalMonitorConfig;
use crate::external_ping::{PingEvent, dispatch_await_any_success, dispatch_external_pings};
use crate::output::{print_error, print_warning};
use crate::ua::user_agent;
use anyhow::Result;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// Maximum size for error body sent by CLI
/// This is a CLI-side limit to avoid huge payloads. Server enforces plan-based limits.
const ERROR_BODY_MAX_BYTES: usize = 100 * 1024;

/// Exit code for monitoring infrastructure failure (distinct from job failure)
/// Using 3 because 2 is commonly used for CLI argument errors
const EXIT_MONITORING_FAILURE: u8 = 3;

/// Maximum timeout for migration mode external check (2 seconds)
const MIGRATION_MODE_TIMEOUT_MS: u64 = 2000;

/// Result of executing a command
struct CommandResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    signal: Option<i32>,
}

/// Execute the monitor command (wrap a command with start/success/fail pings)
///
/// Usage: pakyas monitor <SLUG> -- <COMMAND> [ARGS...]
///
/// Flow:
/// 1. Resolve slug to public_id
/// 2. Send /start ping to pakyas + external monitors (fire-and-forget)
/// 3. Execute command (capture stdout/stderr silently)
/// 4. Send completion ping to pakyas
/// 5. Handle migration mode if pakyas fails
/// 6. Send completion ping to external monitors (fire-and-forget)
/// 7. Exit with the same code as the wrapped command (or 3 for monitoring failure)
pub async fn execute(ctx: &Context, args: MonitorArgs, verbose: bool) -> Result<ExitCode> {
    let project_id = ctx.require_project()?;

    // Validate command
    if args.command.is_empty() {
        return Err(CliError::Other("No command specified".to_string()).into());
    }

    // Resolve slug to public_id
    let public_id = resolve_public_id(ctx, project_id, &args.slug).await?;
    let ping_url = ctx.ping_url();

    // Generate run_id for START/END pairing
    // This enables accurate duration tracking even with concurrent runs
    let run_id = uuid::Uuid::new_v4().to_string();

    // Load external monitor config
    let (monitors, migration_mode) = load_external_config(&args, verbose);

    if verbose {
        eprintln!(
            "[verbose] Loaded {} external monitor(s) for '{}'",
            monitors.len(),
            args.slug
        );
        eprintln!("[verbose] Migration mode: {}", migration_mode);
    }

    // Send start ping to pakyas (with run_id for pairing)
    if verbose {
        eprintln!(
            "[verbose] Sending start ping to pakyas: {}/{}/start",
            ping_url, public_id
        );
    }
    send_ping_direct_inner(&ping_url, public_id, "/start", Some(&run_id)).await?;
    if verbose {
        eprintln!("[verbose] Pakyas start ping succeeded");
    }

    // Send start ping to external monitors (collect handle to await later)
    let start_event = PingEvent::start(&args.slug);
    let start_handle = dispatch_external_pings(
        monitors.clone(),
        start_event,
        args.external_timeout_ms,
        verbose,
    );

    // Execute the wrapped command (capture output silently)
    if verbose {
        eprintln!("[verbose] Executing command: {:?}", args.command);
    }
    let start_time = Instant::now();
    let result = execute_command(&args.command)?;
    let duration_ms = start_time.elapsed().as_millis() as u64;

    if verbose {
        eprintln!(
            "[verbose] Command finished: exit_code={}, duration={}ms",
            result.exit_code, duration_ms
        );
    }

    // Build completion event for external monitors
    let completion_event =
        PingEvent::completion(&args.slug, result.exit_code, duration_ms, &result.stderr);

    // Send completion ping to pakyas (with run_id for pairing)
    if verbose {
        let modifier = if result.exit_code == 0 {
            "".to_string()
        } else {
            format!("/{}", result.exit_code)
        };
        eprintln!(
            "[verbose] Sending completion ping to pakyas: {}/{}{}",
            ping_url, public_id, modifier
        );
    }
    let pakyas_result = send_pakyas_completion(&ping_url, public_id, &result, &run_id).await;

    if verbose {
        match &pakyas_result {
            Ok(_) => eprintln!("[verbose] Pakyas completion ping succeeded"),
            Err(e) => eprintln!("[verbose] Pakyas completion ping failed: {}", e),
        }
    }

    // Handle exit code based on pakyas result and migration mode
    let (exit_code, completion_handle) = match pakyas_result {
        Ok(_) => {
            // Pakyas succeeded - dispatch externals and await before exit
            let handle = dispatch_external_pings(
                monitors,
                completion_event,
                args.external_timeout_ms,
                verbose,
            );
            (ExitCode::from(result.exit_code as u8), handle)
        }
        Err(e) if migration_mode => {
            // Pakyas failed, migration mode: await any external success
            if verbose {
                eprintln!("[verbose] Migration mode: awaiting external monitor success");
            }
            let any_external_success = if monitors.is_empty() {
                false
            } else {
                let timeout = args.external_timeout_ms.min(MIGRATION_MODE_TIMEOUT_MS);
                dispatch_await_any_success(monitors, completion_event, timeout).await
            };

            if any_external_success {
                print_warning(&format!(
                    "Pakyas ping failed ({}), but external monitor succeeded (migration mode)",
                    e
                ));
                // Already awaited in dispatch_await_any_success, no handle to collect
                (ExitCode::from(result.exit_code as u8), None)
            } else {
                print_error(&format!("Pakyas ping failed: {}", e));
                (ExitCode::from(EXIT_MONITORING_FAILURE), None)
            }
        }
        Err(e) => {
            // Pakyas failed, strict mode: exit 3 (monitoring failure)
            // Still notify externals and await before exit
            let handle = dispatch_external_pings(
                monitors,
                completion_event,
                args.external_timeout_ms,
                verbose,
            );
            print_error(&format!("Pakyas ping failed: {}", e));
            (ExitCode::from(EXIT_MONITORING_FAILURE), handle)
        }
    };

    // Await pending external monitor pings before exiting
    await_external_handles(start_handle, completion_handle, args.external_timeout_ms).await;

    Ok(exit_code)
}

/// Await pending external monitor handles with timeout
async fn await_external_handles(
    start_handle: Option<tokio::task::JoinHandle<()>>,
    completion_handle: Option<tokio::task::JoinHandle<()>>,
    timeout_ms: u64,
) {
    let timeout = Duration::from_millis(timeout_ms);

    if let Some(handle) = start_handle {
        if tokio::time::timeout(timeout, handle).await.is_err() {
            eprintln!("Warning: external start ping timed out");
        }
    }

    if let Some(handle) = completion_handle {
        if tokio::time::timeout(timeout, handle).await.is_err() {
            eprintln!("Warning: external completion ping timed out");
        }
    }
}

/// Load external monitor configuration and determine migration mode
fn load_external_config(
    args: &MonitorArgs,
    verbose: bool,
) -> (Vec<crate::external_monitors::MonitorTarget>, bool) {
    if args.no_external {
        if verbose {
            eprintln!("[verbose] External monitors disabled (--no-external)");
        }
        return (vec![], false);
    }

    // Show config paths being checked
    if verbose {
        eprintln!("[verbose] Checking external monitors config paths:");
        for path in ExternalMonitorConfig::config_paths() {
            let exists = path.exists();
            eprintln!("[verbose]   {} (exists: {})", path.display(), exists);
        }
    }

    match ExternalMonitorConfig::load() {
        Ok(config) => {
            if verbose {
                if let Ok(path) = ExternalMonitorConfig::path() {
                    eprintln!("[verbose] Using config: {}", path.display());
                }
            }
            let monitors = config.build_monitors_for_check(&args.slug);
            // CLI flag overrides config file
            let migration_mode = args.migration_mode || config.migration_mode;
            (monitors, migration_mode)
        }
        Err(e) => {
            if verbose {
                eprintln!("[verbose] Failed to load external monitors config: {}", e);
            }
            (vec![], args.migration_mode)
        }
    }
}

/// Send completion ping to pakyas
async fn send_pakyas_completion(
    ping_url: &str,
    public_id: uuid::Uuid,
    result: &CommandResult,
    run_id: &str,
) -> Result<(), anyhow::Error> {
    if result.exit_code == 0 {
        // Success ping (GET, no body)
        send_ping_direct_inner(ping_url, public_id, "", Some(run_id)).await
    } else {
        // Fail ping with error body (POST)
        let modifier = format!("/{}", result.exit_code);
        let error_body = build_error_body(result);
        send_ping_direct_with_body_inner(
            ping_url,
            public_id,
            &modifier,
            Some(&error_body),
            Some(run_id),
        )
        .await
    }
}

/// Send ping directly (returns error instead of swallowing it)
async fn send_ping_direct_inner(
    ping_url: &str,
    public_id: uuid::Uuid,
    modifier: &str,
    run_id: Option<&str>,
) -> Result<(), anyhow::Error> {
    let url = format!(
        "{}/{}{}",
        ping_url.trim_end_matches('/'),
        public_id,
        modifier
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut request = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, user_agent());

    // Add run_id header for START/END pairing
    if let Some(rid) = run_id {
        request = request.header("X-Pakyas-Run", rid);
    }

    let response = request.send().await?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("status {}: {}", status, body)
    }
}

/// Send ping with body directly (returns error instead of swallowing it)
async fn send_ping_direct_with_body_inner(
    ping_url: &str,
    public_id: uuid::Uuid,
    modifier: &str,
    body: Option<&str>,
    run_id: Option<&str>,
) -> Result<(), anyhow::Error> {
    let url = format!(
        "{}/{}{}",
        ping_url.trim_end_matches('/'),
        public_id,
        modifier
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let response = match body {
        Some(b) => {
            let mut request = client
                .post(&url)
                .header(reqwest::header::USER_AGENT, user_agent())
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(b.to_owned());

            // Add run_id header for START/END pairing
            if let Some(rid) = run_id {
                request = request.header("X-Pakyas-Run", rid);
            }

            request.send().await?
        }
        None => {
            let mut request = client
                .get(&url)
                .header(reqwest::header::USER_AGENT, user_agent());

            // Add run_id header for START/END pairing
            if let Some(rid) = run_id {
                request = request.header("X-Pakyas-Run", rid);
            }

            request.send().await?
        }
    };

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("status {}: {}", status, body)
    }
}

/// Execute a command and capture its output
fn execute_command(command: &[String]) -> Result<CommandResult> {
    let program = &command[0];
    let args = &command[1..];

    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped()) // Capture stdout
        .stderr(Stdio::piped()) // Capture stderr
        .output()
        .map_err(|e| CliError::Other(format!("Failed to execute '{}': {}", program, e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let (exit_code, signal) = exit_code_and_signal(&output.status);

    Ok(CommandResult {
        exit_code,
        stdout,
        stderr,
        signal,
    })
}

/// Extract exit code and signal from ExitStatus
fn exit_code_and_signal(status: &std::process::ExitStatus) -> (i32, Option<i32>) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let code = status.code().unwrap_or(1);
        let sig = status.signal();
        (code, sig)
    }

    #[cfg(not(unix))]
    {
        (status.code().unwrap_or(1), None)
    }
}

/// Build error body from command result (stderr preferred, stdout fallback)
fn build_error_body(result: &CommandResult) -> String {
    let mut header = format!("Exit code: {}", result.exit_code);
    if let Some(sig) = result.signal {
        header.push_str(&format!("\nSignal: {}", sig));
    }
    header.push_str("\n---\n");

    // Use stderr if non-empty, otherwise fallback to stdout
    let details = if !result.stderr.trim().is_empty() {
        &result.stderr
    } else {
        &result.stdout
    };

    let mut body = header + details;

    // Truncate by bytes to avoid huge payloads
    if body.len() > ERROR_BODY_MAX_BYTES {
        body.truncate(ERROR_BODY_MAX_BYTES);
        body.push_str("\n…(truncated)\n");
    }

    body
}
