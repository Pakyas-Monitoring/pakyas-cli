//! Doctor command for diagnostic analysis of checks.

use crate::cli::{FailOnSeverity, OutputFormat};
use crate::client::ApiClient;
use crate::config::Context;
use crate::exit_codes;
use crate::output::print_single;
use anyhow::Result;
use chrono::{DateTime, Utc};
use console::style;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::helpers::resolve_check_by_org;

// ============================================================================
// Response Types
// ============================================================================

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

// ============================================================================
// Command Implementation
// ============================================================================

/// Run diagnostic analysis on a check
pub async fn doctor(
    ctx: &Context,
    slug_or_id: &str,
    deep: bool,
    fail_on: FailOnSeverity,
    verbose: bool,
) -> Result<()> {
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
        "healthy" => style("V Healthy").green().to_string(),
        "attention_needed" => style("! Attention needed").yellow().to_string(),
        "critical" => style("X Critical").red().bold().to_string(),
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
            "error" => style("X").red().bold(),
            "warning" => style("!").yellow(),
            "info" => style("i").blue(),
            _ => style("*"),
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
