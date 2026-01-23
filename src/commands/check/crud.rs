//! Basic CRUD operations: list, show, pause, resume, delete, logs, sync.

use crate::cache::CheckCache;
use crate::client::ApiClient;
use crate::config::Context;
use crate::output::{format_status, print_output, print_single, print_success, print_warning};
use anyhow::Result;
use dialoguer::Confirm;

use super::helpers::{
    format_duration, format_ping_type, format_relative_time, resolve_check_by_org,
};
use super::types::{
    CheckDetail, CheckRowWithProject, CheckWithProject, PingHistoryResponse, PingRow,
};

/// List all checks in the organization (optionally filtered by project)
pub async fn list(ctx: &Context, project_filter: Option<&str>, verbose: bool) -> Result<()> {
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

/// Show check details
pub async fn show(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    let org_id = ctx.require_org()?;
    let check = resolve_check_by_org(ctx, org_id, slug_or_id).await?;

    let detail = CheckDetail {
        id: check.id.to_string(),
        public_id: check.public_id.to_string(),
        name: check.name,
        slug: check.slug,
        status: check.status,
        period: format_duration(check.period_seconds),
        grace: format_duration(check.missing_after_seconds),
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
pub async fn pause(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    set_check_state(ctx, slug_or_id, "pause").await
}

/// Resume a paused check
pub async fn resume(ctx: &Context, slug_or_id: &str, _verbose: bool) -> Result<()> {
    set_check_state(ctx, slug_or_id, "resume").await
}

/// Delete a check
pub async fn delete(
    ctx: &Context,
    slug_or_id: &str,
    skip_confirm: bool,
    _verbose: bool,
) -> Result<()> {
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
pub async fn logs(ctx: &Context, slug_or_id: &str, limit: i32, _verbose: bool) -> Result<()> {
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
pub async fn sync(ctx: &Context, _verbose: bool) -> Result<()> {
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
