use crate::cli::{OutputFormat, TimeDisplayMode, TimeZoneMode};
use crate::config::Context;
use chrono::{DateTime, Local, Utc};
use console::{Style, style};
use serde::Serialize;
use std::io::{self, Write};
use tabled::{Table, Tabled};

/// Configuration for output formatting.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub format: OutputFormat,
    pub tz: TimeZoneMode,
    pub time_display: TimeDisplayMode,
    pub no_color: bool,
    pub plain: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Table,
            tz: TimeZoneMode::Local,
            time_display: TimeDisplayMode::Both,
            no_color: false,
            plain: false,
        }
    }
}

impl OutputConfig {
    /// Create from context settings.
    pub fn from_context(ctx: &Context) -> Self {
        Self {
            format: ctx.output_format(),
            tz: ctx.timezone_mode(),
            time_display: ctx.time_display_mode(),
            no_color: ctx.no_color(),
            plain: ctx.plain(),
        }
    }
}

/// Print data as a table
pub fn print_table<T: Tabled>(data: Vec<T>) {
    if data.is_empty() {
        println!("{}", style("No items found").dim());
        return;
    }
    let table = Table::new(data).to_string();
    println!("{}", table);
}

/// Print data as JSON
pub fn print_json<T: Serialize>(data: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(data)?;
    println!("{}", json);
    Ok(())
}

/// Print data as NDJSON (one JSON object per line)
pub fn print_ndjson<T: Serialize>(data: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string(data)?;
    println!("{}", json);
    // Flush to ensure streaming works
    io::stdout().flush()?;
    Ok(())
}

/// Print multiple items as NDJSON (one line per item)
pub fn print_ndjson_stream<T: Serialize>(items: impl IntoIterator<Item = T>) -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    for item in items {
        let json = serde_json::to_string(&item)?;
        writeln!(stdout, "{}", json)?;
    }
    stdout.flush()?;
    Ok(())
}

/// Print data as YAML
pub fn print_yaml<T: Serialize>(data: &T) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(data)?;
    print!("{}", yaml);
    Ok(())
}

/// Print data based on format preference
pub fn print_output<T: Tabled + Serialize>(ctx: &Context, data: Vec<T>) -> anyhow::Result<()> {
    match ctx.output_format() {
        OutputFormat::Table => {
            print_table(data);
        }
        OutputFormat::Json => {
            print_json(&data)?;
        }
        OutputFormat::Ndjson => {
            print_ndjson_stream(data)?;
        }
        OutputFormat::Yaml => {
            print_yaml(&data)?;
        }
    }
    Ok(())
}

/// Print a single item based on format preference
pub fn print_single<T: Serialize>(ctx: &Context, data: &T) -> anyhow::Result<()> {
    match ctx.output_format() {
        OutputFormat::Table => {
            // For single items in table mode, just print JSON pretty
            print_json(data)?;
        }
        OutputFormat::Json => {
            print_json(data)?;
        }
        OutputFormat::Ndjson => {
            print_ndjson(data)?;
        }
        OutputFormat::Yaml => {
            print_yaml(data)?;
        }
    }
    Ok(())
}

/// Print a success message
pub fn print_success(msg: &str) {
    println!("{} {}", style("✓").green().bold(), msg);
}

/// Print an error message
pub fn print_error(msg: &str) {
    eprintln!("{} {}", style("✗").red().bold(), msg);
}

/// Print a warning message
pub fn print_warning(msg: &str) {
    println!("{} {}", style("!").yellow().bold(), msg);
}

/// Print an info message
pub fn print_info(msg: &str) {
    println!("{} {}", style("ℹ").blue().bold(), msg);
}

/// Style for different check statuses
pub fn status_style(status: &str) -> Style {
    match status.to_lowercase().as_str() {
        "up" => Style::new().green(),
        "down" => Style::new().red().bold(),
        "late" | "overrunning" => Style::new().yellow(),
        "running" => Style::new().cyan(),
        "new" | "paused" => Style::new().dim(),
        _ => Style::new(),
    }
}

/// Format a status with appropriate color
pub fn format_status(status: &str) -> String {
    let styled = status_style(status).apply_to(status);
    styled.to_string()
}

// =============================================================================
// Timestamp Formatting
// =============================================================================

/// Format a UTC timestamp according to the output config.
pub fn format_timestamp(dt: DateTime<Utc>, config: &OutputConfig) -> String {
    let formatted = match config.tz {
        TimeZoneMode::Utc => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        TimeZoneMode::Local => {
            let local: DateTime<Local> = dt.into();
            local.format("%Y-%m-%d %H:%M:%S %Z").to_string()
        }
    };

    match config.time_display {
        TimeDisplayMode::Absolute => formatted,
        TimeDisplayMode::Relative => format_relative_time_from_dt(dt),
        TimeDisplayMode::Both => {
            let relative = format_relative_time_from_dt(dt);
            format!("{} ({})", formatted, relative)
        }
    }
}

/// Format a datetime as relative time (e.g., "5m ago", "2h ago").
pub fn format_relative_time_from_dt(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_seconds() < 0 {
        // Future time
        let abs_diff = dt.signed_duration_since(now);
        if abs_diff.num_seconds() < 60 {
            "in <1m".to_string()
        } else if abs_diff.num_minutes() < 60 {
            format!("in {}m", abs_diff.num_minutes())
        } else if abs_diff.num_hours() < 24 {
            format!("in {}h", abs_diff.num_hours())
        } else {
            format!("in {}d", abs_diff.num_days())
        }
    } else if diff.num_seconds() < 60 {
        "just now".to_string()
    } else if diff.num_minutes() < 60 {
        format!("{}m ago", diff.num_minutes())
    } else if diff.num_hours() < 24 {
        format!("{}h ago", diff.num_hours())
    } else {
        format!("{}d ago", diff.num_days())
    }
}

// =============================================================================
// Symbols
// =============================================================================

/// Get success symbol (checkmark or ASCII equivalent).
pub fn symbol_success(plain: bool) -> &'static str {
    if plain { "[OK]" } else { "✓" }
}

/// Get error symbol (X or ASCII equivalent).
pub fn symbol_error(plain: bool) -> &'static str {
    if plain { "[ERROR]" } else { "✗" }
}

/// Get warning symbol (! or ASCII equivalent).
pub fn symbol_warning(plain: bool) -> &'static str {
    if plain { "[WARN]" } else { "!" }
}

/// Get info symbol (i or ASCII equivalent).
pub fn symbol_info(plain: bool) -> &'static str {
    if plain { "[INFO]" } else { "ℹ" }
}

/// Get status symbol based on status name.
pub fn symbol_status(status: &str, plain: bool) -> &'static str {
    match status.to_lowercase().as_str() {
        "up" | "success" => symbol_success(plain),
        "down" | "missing" | "fail" | "error" => symbol_error(plain),
        "late" | "overrunning" | "warning" => symbol_warning(plain),
        "running" => {
            if plain {
                "[RUNNING]"
            } else {
                "▶"
            }
        }
        "new" | "paused" => {
            if plain {
                "[PAUSED]"
            } else {
                "⏸"
            }
        }
        _ => {
            if plain {
                "[-]"
            } else {
                "•"
            }
        }
    }
}

// =============================================================================
// Printing with Config
// =============================================================================

/// Print a success message with config awareness.
pub fn print_success_cfg(msg: &str, config: &OutputConfig) {
    let symbol = symbol_success(config.plain);
    if config.no_color {
        println!("{} {}", symbol, msg);
    } else {
        println!("{} {}", style(symbol).green().bold(), msg);
    }
}

/// Print an error message with config awareness.
pub fn print_error_cfg(msg: &str, config: &OutputConfig) {
    let symbol = symbol_error(config.plain);
    if config.no_color {
        eprintln!("{} {}", symbol, msg);
    } else {
        eprintln!("{} {}", style(symbol).red().bold(), msg);
    }
}

/// Print a warning message with config awareness.
pub fn print_warning_cfg(msg: &str, config: &OutputConfig) {
    let symbol = symbol_warning(config.plain);
    if config.no_color {
        println!("{} {}", symbol, msg);
    } else {
        println!("{} {}", style(symbol).yellow().bold(), msg);
    }
}

/// Print an info message with config awareness.
pub fn print_info_cfg(msg: &str, config: &OutputConfig) {
    let symbol = symbol_info(config.plain);
    if config.no_color {
        println!("{} {}", symbol, msg);
    } else {
        println!("{} {}", style(symbol).blue().bold(), msg);
    }
}
