use crate::cli::OutputFormat;
use crate::config::Context;
use console::{style, Style};
use serde::Serialize;
use tabled::{Table, Tabled};

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

/// Print data based on format preference
pub fn print_output<T: Tabled + Serialize>(ctx: &Context, data: Vec<T>) -> anyhow::Result<()> {
    match ctx.output_format() {
        OutputFormat::Table => {
            print_table(data);
        }
        OutputFormat::Json => {
            print_json(&data)?;
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
