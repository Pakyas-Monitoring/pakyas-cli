//! Exit codes for CLI commands.
//!
//! Provides standardized exit codes for different error conditions,
//! enabling scripting and automation.

/// Success - command completed successfully.
pub const SUCCESS: i32 = 0;

/// Issues found - check has problems (e.g., doctor found errors).
pub const ISSUES: i32 = 1;

/// Usage error - invalid arguments or missing required options.
pub const USAGE: i32 = 2;

/// Not found - requested resource does not exist.
pub const NOT_FOUND: i32 = 3;

/// Network error - API call failed or timeout.
pub const NETWORK: i32 = 4;

/// Authentication error - invalid or expired credentials.
pub const AUTH: i32 = 5;

/// Permission error - insufficient permissions.
pub const PERMISSION: i32 = 6;

/// Internal error - unexpected error occurred.
pub const INTERNAL: i32 = 7;

/// Convert an anyhow::Error to an appropriate exit code.
pub fn from_error(err: &anyhow::Error) -> i32 {
    let msg = err.to_string().to_lowercase();

    if msg.contains("not found") || msg.contains("no check") {
        NOT_FOUND
    } else if msg.contains("unauthorized")
        || msg.contains("invalid token")
        || msg.contains("expired")
    {
        AUTH
    } else if msg.contains("forbidden") || msg.contains("permission") {
        PERMISSION
    } else if msg.contains("timeout") || msg.contains("connection") || msg.contains("network") {
        NETWORK
    } else if msg.contains("usage") || msg.contains("argument") || msg.contains("required") {
        USAGE
    } else {
        INTERNAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn test_from_error_not_found() {
        let err = anyhow!("Check not found");
        assert_eq!(from_error(&err), NOT_FOUND);
    }

    #[test]
    fn test_from_error_auth() {
        let err = anyhow!("Unauthorized: invalid token");
        assert_eq!(from_error(&err), AUTH);
    }

    #[test]
    fn test_from_error_network() {
        let err = anyhow!("Connection timeout");
        assert_eq!(from_error(&err), NETWORK);
    }

    #[test]
    fn test_from_error_internal() {
        let err = anyhow!("Something went wrong");
        assert_eq!(from_error(&err), INTERNAL);
    }
}
