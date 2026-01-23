use anyhow::Result;
use clap::Parser;
use pakyas_cli::cli::{AuthCommands, Cli, Commands, OutputFormat};
use pakyas_cli::commands;
use pakyas_cli::config;
use pakyas_cli::update_cache::{UpdateCache, check_for_updates};
use std::process::ExitCode;
use tokio::task::JoinHandle;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    // Load config with CLI overrides
    let mut ctx = config::Context::load()?;
    if let Some(org) = cli.org.clone() {
        ctx.override_org(org);
    }
    if let Some(project) = cli.project.clone() {
        ctx.override_project(project);
    }
    if let Some(format) = cli.format {
        ctx.set_format(format);
    }
    ctx.set_timezone_mode(cli.display_tz);
    ctx.set_time_display_mode(cli.time);
    ctx.set_no_color(cli.no_color);
    ctx.set_plain(cli.plain);
    ctx.set_debug_http(cli.debug_http);

    // Start update check early (non-blocking) for eligible commands
    let update_handle = if should_check_updates(&cli) {
        let api_url = ctx.api_url();
        Some(tokio::spawn(
            async move { perform_update_check(&api_url).await },
        ))
    } else {
        None
    };

    // Dispatch to command handlers
    let result = execute_command(&cli, ctx).await;

    // After command completes, check for update notice
    if let Some(handle) = update_handle {
        print_update_notice(handle, &cli).await;
    }

    result
}

/// Check if we should perform an update check for this command
fn should_check_updates(cli: &Cli) -> bool {
    // Skip if disabled by flag
    if cli.no_update_check {
        return false;
    }

    // Skip if disabled by env var (treat "1", "true", "yes" as disabled)
    if is_update_check_disabled_by_env() {
        return false;
    }

    // Skip if CI environment
    if std::env::var("CI").is_ok() {
        return false;
    }

    // Skip if stderr is not a TTY
    if !atty::is(atty::Stream::Stderr) {
        return false;
    }

    // Skip for hot-path commands and update (which does its own check)
    !matches!(
        cli.command,
        Commands::Monitor(_)
            | Commands::Ping(_)
            | Commands::Completion { .. }
            | Commands::Update(_)
    )
}

/// Check if PAKYAS_NO_UPDATE_CHECK is set to a truthy value
fn is_update_check_disabled_by_env() -> bool {
    std::env::var("PAKYAS_NO_UPDATE_CHECK")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Perform update check and save result to cache
async fn perform_update_check(api_url: &str) -> Option<UpdateCache> {
    // Check if cache is stale
    let cache = UpdateCache::load();
    if !cache.should_check() {
        return Some(cache);
    }

    // Perform update check
    match check_for_updates(api_url).await {
        Ok(new_cache) => {
            // Save to cache (ignore errors)
            let _ = new_cache.save();
            Some(new_cache)
        }
        Err(_) => {
            // On error, return existing cache (may have valid data)
            Some(cache)
        }
    }
}

/// Print update notice if available
async fn print_update_notice(handle: JoinHandle<Option<UpdateCache>>, cli: &Cli) {
    // Don't print for JSON output mode
    if matches!(cli.format, Some(OutputFormat::Json)) {
        return;
    }

    // Double-check TTY (in case output mode changed)
    if !atty::is(atty::Stream::Stderr) {
        return;
    }

    // Wait for update check to complete
    if let Ok(Some(cache)) = handle.await {
        let current_version = env!("CARGO_PKG_VERSION");
        if let Some(notice) = cache.build_notice(current_version) {
            eprintln!("\n{}", notice);
        }
    }
}

/// Execute the CLI command
async fn execute_command(cli: &Cli, ctx: config::Context) -> Result<ExitCode> {
    let verbose = cli.verbose;

    match &cli.command {
        Commands::Login(args) => {
            commands::auth::login(&ctx, args.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Logout => {
            commands::auth::logout(&ctx, verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Whoami => {
            commands::auth::whoami(&ctx, verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Org(command) => {
            commands::org::handle(&ctx, command.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Project(command) => {
            commands::project::handle(&ctx, command.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Check(command) => {
            commands::check::handle(&ctx, command.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Ping(args) => {
            commands::ping::execute(&ctx, args.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Monitor(args) => {
            // Monitor returns the exit code of the wrapped command
            commands::monitor::execute(&ctx, args.clone(), verbose).await
        }
        Commands::ApiKey(command) => {
            commands::api_key::handle(&ctx, command.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Completion { shell } => {
            commands::completion::generate_completions(*shell)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Update(args) => {
            commands::update::execute(&ctx, args.clone(), verbose).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Auth(auth_cmd) => {
            match auth_cmd {
                AuthCommands::Status => {
                    commands::auth::auth_status(&ctx, verbose).await?;
                }
                AuthCommands::Key(key_cmd) => {
                    commands::auth_key::handle(&ctx, key_cmd.clone(), verbose).await?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
