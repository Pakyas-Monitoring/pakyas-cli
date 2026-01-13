//! External ping dispatcher for sending events to external monitoring services.
//!
//! This module handles sending ping events to healthchecks.io, cronitor, and custom webhooks.
//! It supports fire-and-forget dispatch and awaiting any success for migration mode.

use crate::external_monitors::MonitorTarget;
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use tokio::sync::mpsc;

/// Maximum output size in bytes (4KB)
const OUTPUT_MAX_BYTES: usize = 4 * 1024;

/// Default timeout for external requests in milliseconds
const DEFAULT_TIMEOUT_MS: u64 = 5000;

/// Event type for ping events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Start,
    Success,
    Fail,
}

/// Ping event payload - unified model mapped to each service's format
///
/// The `check_identifier` field contains either a check slug or public_id,
/// depending on how the CLI was invoked.
#[derive(Debug, Clone, Serialize)]
pub struct PingEvent {
    pub check_identifier: String,
    pub event_type: EventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl PingEvent {
    /// Create a start event
    pub fn start(check_identifier: &str) -> Self {
        Self {
            check_identifier: check_identifier.to_string(),
            event_type: EventType::Start,
            exit_code: None,
            duration_ms: None,
            timestamp: Utc::now(),
            host: hostname(),
            output: None,
        }
    }

    /// Create a success event
    pub fn success(check_identifier: &str, duration_ms: u64) -> Self {
        Self {
            check_identifier: check_identifier.to_string(),
            event_type: EventType::Success,
            exit_code: Some(0),
            duration_ms: Some(duration_ms),
            timestamp: Utc::now(),
            host: hostname(),
            output: None,
        }
    }

    /// Create a failure event
    pub fn fail(check_identifier: &str, exit_code: i32, duration_ms: u64, stderr: &str) -> Self {
        Self {
            check_identifier: check_identifier.to_string(),
            event_type: EventType::Fail,
            exit_code: Some(exit_code),
            duration_ms: Some(duration_ms),
            timestamp: Utc::now(),
            host: hostname(),
            output: build_output(stderr),
        }
    }

    /// Create a completion event based on exit code
    pub fn completion(
        check_identifier: &str,
        exit_code: i32,
        duration_ms: u64,
        stderr: &str,
    ) -> Self {
        if exit_code == 0 {
            Self::success(check_identifier, duration_ms)
        } else {
            Self::fail(check_identifier, exit_code, duration_ms, stderr)
        }
    }
}

/// Get hostname for event payload
fn hostname() -> Option<String> {
    hostname::get().ok().and_then(|h| h.into_string().ok())
}

/// Build truncated output from stderr (tail, max 4KB)
fn build_output(stderr: &str) -> Option<String> {
    if stderr.is_empty() {
        return None;
    }

    if stderr.len() <= OUTPUT_MAX_BYTES {
        Some(stderr.to_string())
    } else {
        // Take the last 4KB (tail is more useful for errors)
        let start = stderr.len() - OUTPUT_MAX_BYTES;
        Some(format!("…truncated\n{}", &stderr[start..]))
    }
}

/// Send a ping to a single monitor target
async fn send_to_target(client: &Client, target: &MonitorTarget, event: &PingEvent) -> Result<()> {
    match target {
        MonitorTarget::Healthchecks { endpoint, uuid } => {
            send_healthchecks(client, endpoint, uuid, event).await
        }
        MonitorTarget::Cronitor {
            endpoint,
            api_key,
            monitor_key,
        } => send_cronitor(client, endpoint, api_key, monitor_key, event).await,
        MonitorTarget::Webhook { url } => send_webhook(client, url, event).await,
    }
}

/// Send ping to healthchecks.io
///
/// URL patterns:
/// - Start: {endpoint}/{uuid}/start
/// - Success: {endpoint}/{uuid}
/// - Fail: {endpoint}/{uuid}/fail
async fn send_healthchecks(
    client: &Client,
    endpoint: &str,
    uuid: &str,
    event: &PingEvent,
) -> Result<()> {
    let suffix = match event.event_type {
        EventType::Start => "/start",
        EventType::Success => "",
        EventType::Fail => "/fail",
    };

    let url = format!("{}/{}{}", endpoint.trim_end_matches('/'), uuid, suffix);

    // POST with output body if present, GET otherwise
    let response = if let Some(output) = &event.output {
        client
            .post(&url)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(output.clone())
            .send()
            .await?
    } else {
        client.get(&url).send().await?
    };

    if response.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("healthchecks.io returned status {}", response.status())
    }
}

/// Send ping to cronitor
///
/// URL pattern: {endpoint}/p/{api_key}/{monitor_key}?state={state}&message={output}
async fn send_cronitor(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    monitor_key: &str,
    event: &PingEvent,
) -> Result<()> {
    let state = match event.event_type {
        EventType::Start => "run",
        EventType::Success => "complete",
        EventType::Fail => "fail",
    };

    let mut url = format!(
        "{}/p/{}/{}?state={}",
        endpoint.trim_end_matches('/'),
        api_key,
        monitor_key,
        state
    );

    // Add message parameter if we have output (URL encoded)
    if let Some(output) = &event.output {
        // Truncate message for URL (cronitor has limits)
        let message = if output.len() > 2000 {
            &output[..2000]
        } else {
            output
        };
        url.push_str(&format!("&message={}", urlencoding::encode(message)));
    }

    // Add duration if present
    if let Some(duration) = event.duration_ms {
        url.push_str(&format!("&metric=duration:{}", duration));
    }

    let response = client.get(&url).send().await?;

    if response.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("cronitor returned status {}", response.status())
    }
}

/// Send ping to custom webhook (POST JSON)
async fn send_webhook(client: &Client, url: &str, event: &PingEvent) -> Result<()> {
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(event)
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("webhook returned status {}", response.status())
    }
}

/// Dispatch external pings - returns a JoinHandle that can be awaited
///
/// Returns None if monitors is empty.
/// The returned handle completes when all pings have finished (success or failure).
/// Individual failures are logged as warnings.
/// If verbose is true, logs details about each ping.
pub fn dispatch_external_pings(
    monitors: Vec<MonitorTarget>,
    event: PingEvent,
    timeout_ms: u64,
    verbose: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if monitors.is_empty() {
        if verbose {
            eprintln!(
                "[verbose] No external monitors configured for '{}'",
                event.check_identifier
            );
        }
        return None;
    }

    if verbose {
        eprintln!(
            "[verbose] Dispatching {:?} ping to {} external monitor(s) for '{}'",
            event.event_type,
            monitors.len(),
            event.check_identifier
        );
        for target in &monitors {
            eprintln!("[verbose]   - {}: {}", target.name(), target.display_url());
        }
    }

    let timeout = Duration::from_millis(timeout_ms);

    Some(tokio::spawn(async move {
        let client = match Client::builder().timeout(timeout).build() {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "Warning: failed to create HTTP client for external monitors: {}",
                    e
                );
                return;
            }
        };

        // Spawn all pings concurrently and collect handles
        let handles: Vec<_> = monitors
            .into_iter()
            .map(|target| {
                let client = client.clone();
                let event = event.clone();
                let target_name = target.name();
                let target_url = target.display_url();

                tokio::spawn(async move {
                    match send_to_target(&client, &target, &event).await {
                        Ok(_) => {
                            if verbose {
                                eprintln!(
                                    "[verbose] {} ping succeeded: {}",
                                    target_name, target_url
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: external ping to {} failed: {}", target_name, e);
                        }
                    }
                })
            })
            .collect();

        // Await all pings
        for handle in handles {
            let _ = handle.await;
        }
    }))
}

/// Fire-and-forget dispatch (deprecated - prefer dispatch_external_pings)
///
/// Logs warnings on failure but never blocks or fails the caller.
/// WARNING: May not complete if process exits quickly.
#[deprecated(note = "Use dispatch_external_pings and await the handle instead")]
pub fn dispatch_fire_and_forget(monitors: Vec<MonitorTarget>, event: PingEvent, timeout_ms: u64) {
    let _ = dispatch_external_pings(monitors, event, timeout_ms, false);
}

/// Await any success within timeout (for migration mode)
///
/// Returns true if at least one external monitor succeeded.
/// Returns false immediately if monitors is empty.
pub async fn dispatch_await_any_success(
    monitors: Vec<MonitorTarget>,
    event: PingEvent,
    timeout_ms: u64,
) -> bool {
    if monitors.is_empty() {
        return false;
    }

    let timeout = Duration::from_millis(timeout_ms);
    let client = match Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Use a channel to receive results
    let (tx, mut rx) = mpsc::channel::<bool>(monitors.len());

    for target in monitors {
        let client = client.clone();
        let event = event.clone();
        let tx = tx.clone();

        tokio::spawn(async move {
            let success = send_to_target(&client, &target, &event).await.is_ok();
            let _ = tx.send(success).await;
        });
    }

    // Drop our sender so the channel closes when all tasks complete
    drop(tx);

    // Race: return true as soon as any succeeds, or false if all fail/timeout
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Some(true) => return true,  // At least one succeeded
                    Some(false) => continue,     // This one failed, keep waiting
                    None => return false,        // Channel closed, all failed
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return false;  // Timeout
            }
        }
    }
}

/// Dispatch with default timeout - returns awaitable handle
pub fn dispatch_external_pings_default(
    monitors: Vec<MonitorTarget>,
    event: PingEvent,
    verbose: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    dispatch_external_pings(monitors, event, DEFAULT_TIMEOUT_MS, verbose)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ping_event_start() {
        let event = PingEvent::start("my-check");

        assert_eq!(event.check_identifier, "my-check");
        assert_eq!(event.event_type, EventType::Start);
        assert!(event.exit_code.is_none());
        assert!(event.duration_ms.is_none());
        assert!(event.output.is_none());
    }

    #[test]
    fn test_ping_event_success() {
        let event = PingEvent::success("my-check", 1234);

        assert_eq!(event.check_identifier, "my-check");
        assert_eq!(event.event_type, EventType::Success);
        assert_eq!(event.exit_code, Some(0));
        assert_eq!(event.duration_ms, Some(1234));
        assert!(event.output.is_none());
    }

    #[test]
    fn test_ping_event_fail() {
        let event = PingEvent::fail("my-check", 1, 5678, "error message");

        assert_eq!(event.check_identifier, "my-check");
        assert_eq!(event.event_type, EventType::Fail);
        assert_eq!(event.exit_code, Some(1));
        assert_eq!(event.duration_ms, Some(5678));
        assert_eq!(event.output, Some("error message".to_string()));
    }

    #[test]
    fn test_ping_event_completion_success() {
        let event = PingEvent::completion("my-check", 0, 1000, "");

        assert_eq!(event.event_type, EventType::Success);
        assert_eq!(event.exit_code, Some(0));
    }

    #[test]
    fn test_ping_event_completion_fail() {
        let event = PingEvent::completion("my-check", 1, 1000, "failed");

        assert_eq!(event.event_type, EventType::Fail);
        assert_eq!(event.exit_code, Some(1));
    }

    #[test]
    fn test_build_output_empty() {
        assert!(build_output("").is_none());
    }

    #[test]
    fn test_build_output_small() {
        let output = build_output("small error");
        assert_eq!(output, Some("small error".to_string()));
    }

    #[test]
    fn test_build_output_truncated() {
        let large = "x".repeat(OUTPUT_MAX_BYTES + 1000);
        let output = build_output(&large).unwrap();

        assert!(output.starts_with("…truncated\n"));
        assert!(output.len() <= OUTPUT_MAX_BYTES + 20); // truncated prefix + some buffer
    }

    #[test]
    fn test_dispatch_empty_monitors() {
        // Should return None with empty monitors
        let handle = dispatch_external_pings(vec![], PingEvent::start("test"), 1000, false);
        assert!(handle.is_none());
    }

    #[tokio::test]
    async fn test_await_empty_monitors() {
        let result = dispatch_await_any_success(vec![], PingEvent::start("test"), 1000).await;
        assert!(!result);
    }

    #[test]
    fn test_event_serialization() {
        let event = PingEvent::fail("my-check", 1, 1234, "error");
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"check_identifier\":\"my-check\""));
        assert!(json.contains("\"event_type\":\"fail\""));
        assert!(json.contains("\"exit_code\":1"));
        assert!(json.contains("\"duration_ms\":1234"));
        assert!(json.contains("\"output\":\"error\""));
    }

    #[test]
    fn test_event_serialization_skips_none() {
        let event = PingEvent::start("my-check");
        let json = serde_json::to_string(&event).unwrap();

        // These should not appear because they're None
        assert!(!json.contains("exit_code"));
        assert!(!json.contains("duration_ms"));
        assert!(!json.contains("output"));
    }
}
