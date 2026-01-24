//! Check module for managing cron job monitors.
//!
//! This module provides commands for creating, listing, updating, and monitoring checks.

mod create;
mod crud;
mod doctor;
mod helpers;
mod inspect;
mod tail;
mod types;
mod update;

// Re-export public API used by other modules (ping.rs, monitor.rs)
pub use helpers::{resolve_public_id, resolve_public_id_smart, resolve_public_id_verbose};
pub use types::{Check, CheckWithProject};

use crate::cli::CheckCommands;
use crate::config::Context;
use anyhow::Result;

/// Handle check subcommands
pub async fn handle(ctx: &Context, command: CheckCommands, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("[verbose] API URL: {}", ctx.api_url());
        if let Some(project) = ctx.active_project_name() {
            eprintln!("[verbose] Active project: {}", project);
        }
    }

    match command {
        CheckCommands::List { project } => crud::list(ctx, project.as_deref(), verbose).await,
        CheckCommands::Create {
            slug,
            name,
            cron,
            tz,
            every,
            missing_after,
            description,
            tags,
            alert_after_miss_pings,
            alert_after_fail_pings,
            max_runtime,
            json,
            quiet,
            dry_run,
            interactive,
        } => {
            create::create(
                ctx,
                slug,
                name,
                cron,
                tz,
                every,
                missing_after,
                description,
                tags,
                alert_after_miss_pings,
                alert_after_fail_pings,
                max_runtime,
                json,
                quiet,
                dry_run,
                interactive,
                verbose,
            )
            .await
        }
        CheckCommands::Show { slug } => crud::show(ctx, &slug, verbose).await,
        CheckCommands::Pause { slug } => crud::pause(ctx, &slug, verbose).await,
        CheckCommands::Resume { slug } => crud::resume(ctx, &slug, verbose).await,
        CheckCommands::Delete { slug, yes } => crud::delete(ctx, &slug, yes, verbose).await,
        CheckCommands::Logs { slug, limit } => crud::logs(ctx, &slug, limit, verbose).await,
        CheckCommands::Sync => crud::sync(ctx, verbose).await,
        CheckCommands::Update {
            slug,
            name,
            description,
            cron,
            tz,
            every,
            missing_after,
            tags,
            alert_after_miss_pings,
            alert_after_fail_pings,
            max_runtime,
            yes,
        } => {
            update::update(
                ctx,
                &slug,
                name,
                description,
                cron,
                tz,
                every,
                missing_after,
                tags,
                alert_after_miss_pings,
                alert_after_fail_pings,
                max_runtime,
                yes,
                verbose,
            )
            .await
        }
        CheckCommands::Inspect { slug } => inspect::inspect(ctx, &slug, verbose).await,
        CheckCommands::Doctor {
            slug,
            deep,
            fail_on,
        } => doctor::doctor(ctx, &slug, deep, fail_on, verbose).await,
        CheckCommands::Tail {
            slug,
            since,
            types,
            follow,
            limit,
        } => tail::tail(ctx, &slug, &since, types.as_deref(), follow, limit, verbose).await,
    }
}
