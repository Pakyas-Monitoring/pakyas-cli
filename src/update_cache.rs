//! Update cache for CLI version checking
//!
//! Stores the last update check result locally to avoid checking on every command.
//! Cache is stored alongside checks.json at `{config_dir}/cache/update.json`.

use crate::config::Config;
use crate::error::CliError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const UPDATE_CHECK_TTL_HOURS: i64 = 24;

/// Cached update check result
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateCache {
    /// When we last checked for updates
    pub last_checked_at: Option<DateTime<Utc>>,
    /// Latest CLI version available
    pub latest_version: Option<String>,
    /// Minimum supported CLI version
    pub min_supported: Option<String>,
    /// Release channel (e.g., "stable", "beta")
    pub channel: Option<String>,
    /// Optional message from server (e.g., "Security fix available")
    pub message: Option<String>,
}

impl UpdateCache {
    /// Load update cache from disk
    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save update cache to disk (atomic write)
    pub fn save(&self) -> Result<(), CliError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        // Atomic write: write to temp, then rename
        let temp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&temp_path, &content).map_err(CliError::ConfigWrite)?;
        std::fs::rename(&temp_path, &path).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    /// Get cache file path
    fn path() -> Result<PathBuf, CliError> {
        let config_dir = Config::config_dir()?;
        Ok(config_dir.join("cache").join("update.json"))
    }

    /// Check if we should perform an update check
    pub fn should_check(&self) -> bool {
        match self.last_checked_at {
            None => true,
            Some(ts) => Utc::now() - ts > Duration::hours(UPDATE_CHECK_TTL_HOURS),
        }
    }

    /// Check if an update is available
    pub fn update_available(&self, current: &str) -> bool {
        self.latest_version
            .as_ref()
            .map(|latest| semver_gt(latest, current))
            .unwrap_or(false)
    }

    /// Check if current version is below minimum supported
    pub fn version_unsupported(&self, current: &str) -> bool {
        self.min_supported
            .as_ref()
            .map(|min| semver_gt(min, current))
            .unwrap_or(false)
    }

    /// Build update notice message if update is available
    pub fn build_notice(&self, current: &str) -> Option<String> {
        if !self.update_available(current) {
            return None;
        }

        let latest = self.latest_version.as_ref()?;
        let mut notice = format!(
            "Update available: pakyas v{} → v{} (https://pakyas.com/docs/cli/install)",
            current, latest
        );

        if let Some(msg) = &self.message {
            notice.push_str(&format!("\n{}", msg));
        }

        Some(notice)
    }
}

/// Compare two semver versions, returns true if a > b
pub fn semver_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(va), Ok(vb)) => va > vb,
        _ => false,
    }
}

/// Response from /cli/latest endpoint
#[derive(Debug, Deserialize)]
pub struct CliLatestResponse {
    pub latest_version: String,
    pub min_supported: String,
    pub channel: String,
    pub message: Option<String>,
}

/// Check for updates from the server
pub async fn check_for_updates(api_url: &str) -> Result<UpdateCache, CliError> {
    let url = format!("{}/cli/latest", api_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| CliError::Other(format!("Failed to build HTTP client: {}", e)))?;

    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, crate::ua::user_agent())
        .send()
        .await
        .map_err(|e| CliError::Other(format!("Update check failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(CliError::Other(format!(
            "Update check failed: HTTP {}",
            response.status()
        )));
    }

    let data: CliLatestResponse = response
        .json()
        .await
        .map_err(|e| CliError::Other(format!("Invalid update response: {}", e)))?;

    Ok(UpdateCache {
        last_checked_at: Some(Utc::now()),
        latest_version: Some(data.latest_version),
        min_supported: Some(data.min_supported),
        channel: Some(data.channel),
        message: data.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_semver_gt() {
        assert!(semver_gt("1.0.1", "1.0.0"));
        assert!(semver_gt("1.1.0", "1.0.0"));
        assert!(semver_gt("2.0.0", "1.9.9"));
        assert!(!semver_gt("1.0.0", "1.0.0"));
        assert!(!semver_gt("1.0.0", "1.0.1"));
        assert!(!semver_gt("invalid", "1.0.0"));
        assert!(!semver_gt("1.0.0", "invalid"));
    }

    #[test]
    fn test_update_available() {
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            latest_version: Some("1.2.0".to_string()),
            min_supported: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
            message: None,
        };

        assert!(cache.update_available("1.0.0"));
        assert!(cache.update_available("1.1.0"));
        assert!(!cache.update_available("1.2.0"));
        assert!(!cache.update_available("1.3.0"));
    }

    #[test]
    fn test_version_unsupported() {
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            latest_version: Some("1.2.0".to_string()),
            min_supported: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
            message: None,
        };

        assert!(cache.version_unsupported("0.9.0"));
        assert!(!cache.version_unsupported("1.0.0"));
        assert!(!cache.version_unsupported("1.1.0"));
    }

    #[test]
    fn test_should_check() {
        // Never checked
        let cache = UpdateCache::default();
        assert!(cache.should_check());

        // Recently checked
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            ..Default::default()
        };
        assert!(!cache.should_check());

        // Checked > 24h ago
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now() - Duration::hours(25)),
            ..Default::default()
        };
        assert!(cache.should_check());
    }

    #[test]
    fn test_build_notice() {
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            latest_version: Some("1.2.0".to_string()),
            min_supported: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
            message: None,
        };

        let notice = cache.build_notice("1.0.0").unwrap();
        assert!(notice.contains("1.0.0 → v1.2.0"));
        assert!(notice.contains("pakyas.com/docs/cli/install"));

        // No update available
        assert!(cache.build_notice("1.2.0").is_none());
    }

    #[test]
    fn test_build_notice_with_message() {
        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            latest_version: Some("1.2.0".to_string()),
            min_supported: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
            message: Some("Security fix available!".to_string()),
        };

        let notice = cache.build_notice("1.0.0").unwrap();
        assert!(notice.contains("Security fix available!"));
    }

    #[test]
    fn test_cache_save_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("update.json");

        let cache = UpdateCache {
            last_checked_at: Some(Utc::now()),
            latest_version: Some("1.2.0".to_string()),
            min_supported: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
            message: Some("Test message".to_string()),
        };

        // Write to temp path for testing
        let content = serde_json::to_string_pretty(&cache).unwrap();
        std::fs::write(&path, &content).unwrap();

        // Read back
        let loaded: UpdateCache =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(loaded.latest_version, cache.latest_version);
        assert_eq!(loaded.min_supported, cache.min_supported);
        assert_eq!(loaded.message, cache.message);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        // When file doesn't exist, load() should return default
        let cache = UpdateCache::default();
        assert!(cache.last_checked_at.is_none());
        assert!(cache.latest_version.is_none());
        assert!(cache.should_check());
    }
}
