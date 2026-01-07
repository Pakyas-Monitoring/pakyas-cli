//! Build script for pakyas-cli
//!
//! Injects build-time defaults for API_URL and PING_URL from environment variables.
//! These can be overridden at runtime via the same environment variables.

fn main() {
    // Load .env file if present (for local development)
    let _ = dotenvy::dotenv();

    // Provide production defaults for open-source builds
    // Override via environment variables or .env file
    let api_url = std::env::var("API_URL")
        .unwrap_or_else(|_| "https://api.pakyas.com".to_string());

    let ping_url = std::env::var("PING_URL")
        .unwrap_or_else(|_| "https://ping.pakyas.com".to_string());

    // Pass to compiler as compile-time env vars
    println!("cargo:rustc-env=API_URL={}", api_url);
    println!("cargo:rustc-env=PING_URL={}", ping_url);

    // Re-run if these change
    println!("cargo:rerun-if-env-changed=API_URL");
    println!("cargo:rerun-if-env-changed=PING_URL");
    println!("cargo:rerun-if-changed=.env");
}
