//! Self-update command for pakyas CLI.
//!
//! Downloads and replaces the CLI binary with the latest version from Tigris storage.

use crate::cli::UpdateArgs;
use crate::config::Context;
use crate::output::{print_error, print_info, print_success};
use crate::ua::user_agent;
use crate::update_cache::{check_for_updates, semver_gt};
use anyhow::{Result, anyhow};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use tar::Archive;

/// Execute the update command
pub async fn execute(ctx: &Context, args: UpdateArgs, verbose: bool) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let api_url = ctx.api_url();

    if verbose {
        eprintln!("[verbose] Current version: {}", current_version);
        eprintln!("[verbose] API URL: {}", api_url);
    }

    // Fetch latest version info
    print_info("Checking for updates...");
    let latest = check_for_updates(&api_url).await?;

    let latest_version = latest
        .latest_version
        .ok_or_else(|| anyhow!("Could not determine latest version"))?;

    // Check if update needed
    if !semver_gt(&latest_version, current_version) {
        print_success(&format!("Already up to date (v{})", current_version));
        return Ok(());
    }

    // If --check flag, just report and exit
    if args.check {
        println!(
            "Update available: v{} → v{}",
            current_version, latest_version
        );
        if let Some(msg) = &latest.message {
            println!("{}", msg);
        }
        return Ok(());
    }

    // Get current binary path
    let current_exe = std::env::current_exe()?;
    let target_path = current_exe.canonicalize()?;

    // Check write permission
    if !can_write(&target_path) {
        print_error(&format!(
            "Permission denied: {}\n\nTry: sudo pakyas update",
            target_path.display()
        ));
        return Err(anyhow!("Permission denied"));
    }

    // Detect platform
    let target = detect_target()?;

    // Build URLs
    let archive_url = build_download_url(&api_url, &latest_version, target);
    let checksum_url = format!("{}.sha256", archive_url);

    // Download and verify
    print_info(&format!("Downloading pakyas v{}...", latest_version));
    let archive_bytes = download_and_verify(&archive_url, &checksum_url).await?;

    // Extract and replace
    print_info("Installing...");
    extract_and_replace(&archive_bytes, &target_path)?;

    print_success(&format!(
        "Updated pakyas: v{} → v{}",
        current_version, latest_version
    ));

    Ok(())
}

/// Detect Rust target triple from runtime
fn detect_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        (os, arch) => Err(anyhow!("Unsupported platform: {}-{}", os, arch)),
    }
}

/// Build download URL for the CLI binary
fn build_download_url(api_url: &str, version: &str, target: &str) -> String {
    format!(
        "{}/cli/download/{}/pakyas-{}-{}.tar.gz",
        api_url.trim_end_matches('/'),
        version,
        version,
        target
    )
}

/// Check if we have write permission to a path
fn can_write(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

/// Download archive and verify checksum
async fn download_and_verify(archive_url: &str, checksum_url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    // Download archive with progress
    let response = client
        .get(archive_url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow!("Failed to download: HTTP {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut bytes = Vec::with_capacity(total_size as usize);
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        pb.inc(chunk.len() as u64);
        bytes.extend_from_slice(&chunk);
    }

    pb.finish_and_clear();

    // Download checksum
    let checksum_response = client
        .get(checksum_url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .send()
        .await?;

    if !checksum_response.status().is_success() {
        return Err(anyhow!(
            "Failed to download checksum: HTTP {}",
            checksum_response.status()
        ));
    }

    let checksum_text = checksum_response.text().await?;
    let expected_checksum = checksum_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("Invalid checksum format"))?
        .to_lowercase();

    // Verify checksum
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual_checksum = format!("{:x}", hasher.finalize());

    if actual_checksum != expected_checksum {
        return Err(anyhow!(
            "Checksum mismatch!\nExpected: {}\nActual: {}\n\nThe download may be corrupted. Please try again.",
            expected_checksum,
            actual_checksum
        ));
    }

    Ok(bytes)
}

/// Extract binary from archive and replace current executable
fn extract_and_replace(archive_bytes: &[u8], target_path: &Path) -> Result<()> {
    // Extract to temp directory
    let temp_dir = tempfile::tempdir()?;
    let decoder = GzDecoder::new(archive_bytes);
    let mut archive = Archive::new(decoder);
    archive.unpack(&temp_dir)?;

    // Find the pakyas binary in extracted files
    let new_binary = temp_dir.path().join("pakyas");
    if !new_binary.exists() {
        return Err(anyhow!(
            "Binary not found in archive. Expected 'pakyas' at root level."
        ));
    }

    // Verify it's actually an executable (basic sanity check)
    let mut file = std::fs::File::open(&new_binary)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;

    // Check for ELF (Linux) or Mach-O (macOS) magic bytes
    let is_elf = magic == [0x7f, b'E', b'L', b'F'];
    let is_macho = magic == [0xcf, 0xfa, 0xed, 0xfe] || magic == [0xfe, 0xed, 0xfa, 0xcf];
    let is_macho_64 = magic == [0xcf, 0xfa, 0xed, 0xfe]; // 64-bit Mach-O
    let is_macho_universal = magic == [0xca, 0xfe, 0xba, 0xbe]; // Universal binary

    if !is_elf && !is_macho && !is_macho_64 && !is_macho_universal {
        return Err(anyhow!(
            "Downloaded file does not appear to be a valid executable"
        ));
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_binary, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic replace
    let backup_path = target_path.with_extension("old");

    // Remove old backup if exists
    let _ = std::fs::remove_file(&backup_path);

    // Rename current to backup
    std::fs::rename(target_path, &backup_path).map_err(|e| {
        anyhow!(
            "Failed to backup current binary: {}. You may need to run with sudo.",
            e
        )
    })?;

    // Move new binary to target
    if let Err(e) = std::fs::rename(&new_binary, target_path) {
        // Rollback on failure
        let _ = std::fs::rename(&backup_path, target_path);
        return Err(anyhow!(
            "Failed to install new binary: {}. Rolled back to previous version.",
            e
        ));
    }

    // Remove backup on success
    let _ = std::fs::remove_file(&backup_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_target() {
        let result = detect_target();
        // Should succeed on supported platforms
        assert!(
            result.is_ok()
                || cfg!(not(any(
                    all(target_os = "macos", target_arch = "x86_64"),
                    all(target_os = "macos", target_arch = "aarch64"),
                    all(target_os = "linux", target_arch = "x86_64"),
                    all(target_os = "linux", target_arch = "aarch64"),
                )))
        );
    }

    #[test]
    fn test_build_download_url() {
        let url = build_download_url("https://api.pakyas.com", "1.0.0", "x86_64-apple-darwin");
        assert_eq!(
            url,
            "https://api.pakyas.com/cli/download/1.0.0/pakyas-1.0.0-x86_64-apple-darwin.tar.gz"
        );

        // With trailing slash
        let url = build_download_url(
            "https://api.pakyas.com/",
            "1.0.0",
            "aarch64-unknown-linux-gnu",
        );
        assert_eq!(
            url,
            "https://api.pakyas.com/cli/download/1.0.0/pakyas-1.0.0-aarch64-unknown-linux-gnu.tar.gz"
        );
    }
}
