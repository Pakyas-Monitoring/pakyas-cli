//! User-Agent helper for CLI requests
//!
//! Provides a consistent User-Agent string for all HTTP requests made by the CLI.
//! Format: `pakyas-cli/{version} ({os}; {arch})`

use std::sync::OnceLock;

static USER_AGENT: OnceLock<String> = OnceLock::new();

/// Returns the User-Agent string for CLI requests.
///
/// Format: `pakyas-cli/0.1.0 (macos; aarch64)`
///
/// This value is computed once and cached for the lifetime of the process.
pub fn user_agent() -> &'static str {
    USER_AGENT.get_or_init(|| {
        format!(
            "pakyas-cli/{} ({}; {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_agent_format() {
        let ua = user_agent();
        assert!(ua.starts_with("pakyas-cli/"));
        assert!(ua.contains("("));
        assert!(ua.contains(";"));
        assert!(ua.contains(")"));
    }

    #[test]
    fn test_user_agent_contains_version() {
        let ua = user_agent();
        assert!(ua.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn test_user_agent_contains_os_arch() {
        let ua = user_agent();
        assert!(ua.contains(std::env::consts::OS));
        assert!(ua.contains(std::env::consts::ARCH));
    }
}
