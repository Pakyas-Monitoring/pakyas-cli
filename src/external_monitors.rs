//! External monitor configuration for integration with healthchecks.io, cronitor, and webhooks.
//!
//! This module handles loading and merging configuration for external monitoring services,
//! allowing pakyas-cli to ping multiple services in parallel during migrations.

use crate::config::Config;
use crate::error::CliError;
use directories::BaseDirs;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Default healthchecks.io endpoint
const DEFAULT_HEALTHCHECKS_ENDPOINT: &str = "https://hc-ping.com";

/// Default cronitor telemetry endpoint
const DEFAULT_CRONITOR_ENDPOINT: &str = "https://cronitor.link";

/// Root configuration loaded from external_monitors.toml
#[derive(Debug, Deserialize, Default)]
pub struct ExternalMonitorsFile {
    /// Global migration mode setting
    #[serde(default)]
    pub migration_mode: bool,

    /// Global target settings (endpoints, api keys)
    #[serde(default)]
    pub targets: GlobalTargets,

    /// Per-check target configurations
    #[serde(default)]
    pub checks: HashMap<String, CheckTargets>,
}

/// Global target settings - shared across all checks
#[derive(Debug, Deserialize, Default)]
pub struct GlobalTargets {
    #[serde(default)]
    pub healthchecks: Option<GlobalHealthchecks>,

    #[serde(default)]
    pub cronitor: Option<GlobalCronitor>,

    #[serde(default)]
    pub webhook: Option<GlobalWebhook>,
}

/// Global healthchecks settings (endpoint only, no uuid)
#[derive(Debug, Deserialize, Clone)]
pub struct GlobalHealthchecks {
    #[serde(default = "default_healthchecks_endpoint")]
    pub endpoint: String,
}

fn default_healthchecks_endpoint() -> String {
    DEFAULT_HEALTHCHECKS_ENDPOINT.to_string()
}

/// Global cronitor settings (api_key only, no monitor_key)
#[derive(Debug, Deserialize, Clone)]
pub struct GlobalCronitor {
    pub api_key: String,

    #[serde(default = "default_cronitor_endpoint")]
    pub endpoint: String,
}

fn default_cronitor_endpoint() -> String {
    DEFAULT_CRONITOR_ENDPOINT.to_string()
}

/// Global webhook settings
#[derive(Debug, Deserialize, Clone)]
pub struct GlobalWebhook {
    pub url: String,
}

/// Per-check target configurations
#[derive(Debug, Deserialize, Default)]
pub struct CheckTargets {
    #[serde(default)]
    pub targets: CheckTargetIds,
}

/// Per-check target IDs
#[derive(Debug, Deserialize, Default)]
pub struct CheckTargetIds {
    #[serde(default)]
    pub healthchecks: Option<CheckHealthchecks>,

    #[serde(default)]
    pub cronitor: Option<CheckCronitor>,
}

/// Per-check healthchecks config (uuid required)
#[derive(Debug, Deserialize, Clone)]
pub struct CheckHealthchecks {
    pub uuid: String,
}

/// Per-check cronitor config (monitor_key required)
#[derive(Debug, Deserialize, Clone)]
pub struct CheckCronitor {
    pub monitor_key: String,
}

/// Resolved monitor target - ready to send pings
#[derive(Debug, Clone)]
pub enum MonitorTarget {
    Healthchecks { endpoint: String, uuid: String },
    Cronitor { endpoint: String, api_key: String, monitor_key: String },
    Webhook { url: String },
}

impl MonitorTarget {
    /// Get the name of this monitor target for logging
    pub fn name(&self) -> &'static str {
        match self {
            MonitorTarget::Healthchecks { .. } => "healthchecks.io",
            MonitorTarget::Cronitor { .. } => "cronitor",
            MonitorTarget::Webhook { .. } => "webhook",
        }
    }

    /// Get a display URL for verbose logging (hides sensitive parts)
    pub fn display_url(&self) -> String {
        match self {
            MonitorTarget::Healthchecks { endpoint, uuid } => {
                format!("{}/{}", endpoint, uuid)
            }
            MonitorTarget::Cronitor { endpoint, monitor_key, .. } => {
                // Hide API key
                format!("{}/p/***/{}", endpoint, monitor_key)
            }
            MonitorTarget::Webhook { url } => url.clone(),
        }
    }
}

/// Loaded and resolved external monitor configuration
#[derive(Debug)]
pub struct ExternalMonitorConfig {
    pub migration_mode: bool,
    file_config: ExternalMonitorsFile,
}

impl ExternalMonitorConfig {
    /// Load external monitor configuration from the default path
    pub fn load() -> Result<Self, CliError> {
        let path = Self::path()?;
        Self::load_from_path(&path)
    }

    /// Load external monitor configuration from a specific path
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        if !path.exists() {
            return Ok(Self {
                migration_mode: false,
                file_config: ExternalMonitorsFile::default(),
            });
        }

        let content = std::fs::read_to_string(path).map_err(CliError::ConfigRead)?;
        let file_config: ExternalMonitorsFile = toml::from_str(&content)?;

        // Check for migration_mode env var override
        let migration_mode = std::env::var("PAKYAS_MIGRATION_MODE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(file_config.migration_mode);

        Ok(Self {
            migration_mode,
            file_config,
        })
    }

    /// Get the path to the external monitors config file
    /// Checks in order: ~/.config/pakyas/, then system config dir
    pub fn path() -> Result<PathBuf, CliError> {
        // First check ~/.config/pakyas/ (XDG standard location)
        if let Some(base_dirs) = BaseDirs::new() {
            let xdg_path = base_dirs
                .home_dir()
                .join(".config/pakyas/external_monitors.toml");
            if xdg_path.exists() {
                return Ok(xdg_path);
            }
        }

        // Fall back to system config dir
        let config_dir = Config::config_dir()?;
        Ok(config_dir.join("external_monitors.toml"))
    }

    /// Get all potential config paths (for verbose logging)
    pub fn config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        if let Some(base_dirs) = BaseDirs::new() {
            paths.push(
                base_dirs
                    .home_dir()
                    .join(".config/pakyas/external_monitors.toml"),
            );
        }

        if let Ok(config_dir) = Config::config_dir() {
            paths.push(config_dir.join("external_monitors.toml"));
        }

        paths
    }

    /// Build monitor targets for a specific check slug
    ///
    /// Resolution order:
    /// 1. Global settings (endpoint, api_key, webhook url)
    /// 2. Per-check IDs (uuid, monitor_key)
    /// 3. Merge: global settings + per-check IDs = complete target
    /// 4. No ID configured = service skipped
    pub fn build_monitors_for_check(&self, check_slug: &str) -> Vec<MonitorTarget> {
        let mut targets = Vec::new();

        // Get per-check config if exists
        let check_config = self.file_config.checks.get(check_slug);

        // Healthchecks.io: needs global endpoint + per-check uuid
        if let Some(check_hc) = check_config.and_then(|c| c.targets.healthchecks.as_ref()) {
            let endpoint = self
                .file_config
                .targets
                .healthchecks
                .as_ref()
                .map(|g| g.endpoint.clone())
                .or_else(|| std::env::var("HEALTHCHECKS_ENDPOINT").ok())
                .unwrap_or_else(default_healthchecks_endpoint);

            targets.push(MonitorTarget::Healthchecks {
                endpoint,
                uuid: check_hc.uuid.clone(),
            });
        }

        // Cronitor: needs global api_key + per-check monitor_key
        if let Some(check_cr) = check_config.and_then(|c| c.targets.cronitor.as_ref()) {
            // api_key from config or env
            let api_key = self
                .file_config
                .targets
                .cronitor
                .as_ref()
                .map(|g| g.api_key.clone())
                .or_else(|| std::env::var("CRONITOR_API_KEY").ok());

            if let Some(api_key) = api_key {
                let endpoint = self
                    .file_config
                    .targets
                    .cronitor
                    .as_ref()
                    .map(|g| g.endpoint.clone())
                    .unwrap_or_else(default_cronitor_endpoint);

                targets.push(MonitorTarget::Cronitor {
                    endpoint,
                    api_key,
                    monitor_key: check_cr.monitor_key.clone(),
                });
            }
        }

        // Webhook: global only, check slug included in payload
        if let Some(webhook) = self.file_config.targets.webhook.as_ref() {
            targets.push(MonitorTarget::Webhook {
                url: webhook.url.clone(),
            });
        } else if let Ok(url) = std::env::var("EXTERNAL_WEBHOOK_URL") {
            targets.push(MonitorTarget::Webhook { url });
        }

        targets
    }

    /// Check if any external monitors are configured
    pub fn has_any_monitors(&self) -> bool {
        self.file_config.targets.webhook.is_some()
            || std::env::var("EXTERNAL_WEBHOOK_URL").is_ok()
            || !self.file_config.checks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();

        assert!(!config.migration_mode);
        assert!(config.build_monitors_for_check("any-check").is_empty());
    }

    #[test]
    fn test_load_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(&path, "").unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();

        assert!(!config.migration_mode);
        assert!(config.build_monitors_for_check("any-check").is_empty());
    }

    #[test]
    fn test_load_migration_mode() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(&path, "migration_mode = true").unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();

        assert!(config.migration_mode);
    }

    #[test]
    fn test_load_webhook_global() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
[targets.webhook]
url = "https://my-webhook.example.com/ping"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();
        let targets = config.build_monitors_for_check("any-check");

        assert_eq!(targets.len(), 1);
        match &targets[0] {
            MonitorTarget::Webhook { url } => {
                assert_eq!(url, "https://my-webhook.example.com/ping");
            }
            _ => panic!("Expected Webhook target"),
        }
    }

    #[test]
    fn test_load_healthchecks_per_check() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
[checks."backup-db".targets.healthchecks]
uuid = "550e8400-e29b-41d4-a716-446655440000"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();

        // Check with matching slug
        let targets = config.build_monitors_for_check("backup-db");
        assert_eq!(targets.len(), 1);
        match &targets[0] {
            MonitorTarget::Healthchecks { endpoint, uuid } => {
                assert_eq!(endpoint, DEFAULT_HEALTHCHECKS_ENDPOINT);
                assert_eq!(uuid, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("Expected Healthchecks target"),
        }

        // Check with non-matching slug
        let targets = config.build_monitors_for_check("other-check");
        assert!(targets.is_empty());
    }

    #[test]
    fn test_load_healthchecks_with_custom_endpoint() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
[targets.healthchecks]
endpoint = "https://hc.internal.example.com"

[checks."backup-db".targets.healthchecks]
uuid = "test-uuid"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();
        let targets = config.build_monitors_for_check("backup-db");

        assert_eq!(targets.len(), 1);
        match &targets[0] {
            MonitorTarget::Healthchecks { endpoint, uuid } => {
                assert_eq!(endpoint, "https://hc.internal.example.com");
                assert_eq!(uuid, "test-uuid");
            }
            _ => panic!("Expected Healthchecks target"),
        }
    }

    #[test]
    fn test_load_cronitor_per_check() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
[targets.cronitor]
api_key = "test-api-key"

[checks."payment-sync".targets.cronitor]
monitor_key = "payment-sync-monitor"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();
        let targets = config.build_monitors_for_check("payment-sync");

        assert_eq!(targets.len(), 1);
        match &targets[0] {
            MonitorTarget::Cronitor {
                endpoint,
                api_key,
                monitor_key,
            } => {
                assert_eq!(endpoint, DEFAULT_CRONITOR_ENDPOINT);
                assert_eq!(api_key, "test-api-key");
                assert_eq!(monitor_key, "payment-sync-monitor");
            }
            _ => panic!("Expected Cronitor target"),
        }
    }

    #[test]
    fn test_load_multiple_targets() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
[targets.cronitor]
api_key = "cronitor-key"

[targets.webhook]
url = "https://webhook.example.com"

[checks."my-job".targets.healthchecks]
uuid = "hc-uuid"

[checks."my-job".targets.cronitor]
monitor_key = "my-job-monitor"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();
        let targets = config.build_monitors_for_check("my-job");

        // Should have healthchecks, cronitor, and webhook
        assert_eq!(targets.len(), 3);

        let names: Vec<_> = targets.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"healthchecks.io"));
        assert!(names.contains(&"cronitor"));
        assert!(names.contains(&"webhook"));
    }

    #[test]
    fn test_cronitor_without_api_key_skipped() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("external_monitors.toml");
        std::fs::write(
            &path,
            r#"
# No [targets.cronitor] with api_key

[checks."my-job".targets.cronitor]
monitor_key = "my-job-monitor"
"#,
        )
        .unwrap();

        let config = ExternalMonitorConfig::load_from_path(&path).unwrap();
        let targets = config.build_monitors_for_check("my-job");

        // Cronitor should be skipped because no api_key
        assert!(targets.is_empty());
    }
}
