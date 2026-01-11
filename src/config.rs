use crate::cli::OutputFormat;
use crate::error::CliError;
use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Build-time defaults injected via build.rs
// These can be customized at build time via API_URL and PING_URL env vars
const DEFAULT_API_URL: &str = env!("API_URL");
const DEFAULT_PING_URL: &str = env!("PING_URL");

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_api_url")]
    pub api_url: String,

    #[serde(default = "default_ping_url")]
    pub ping_url: String,

    pub active_org_id: Option<String>,
    pub active_org_name: Option<String>,
    /// Cached timezone from active org (IANA format). Used for dry-run/JSON output.
    pub active_org_timezone: Option<String>,

    pub active_project_id: Option<String>,
    pub active_project_name: Option<String>,

    #[serde(default)]
    pub format: String,

    #[serde(default = "default_true")]
    pub color: bool,
}

fn default_api_url() -> String {
    DEFAULT_API_URL.to_string()
}

fn default_ping_url() -> String {
    DEFAULT_PING_URL.to_string()
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn load() -> Result<Self, CliError> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path).map_err(CliError::ConfigRead)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<(), CliError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    pub fn path() -> Result<PathBuf, CliError> {
        let dirs = ProjectDirs::from("com", "pakyas", "pakyas")
            .ok_or_else(|| CliError::Other("Could not determine config directory".to_string()))?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    pub fn config_dir() -> Result<PathBuf, CliError> {
        let dirs = ProjectDirs::from("com", "pakyas", "pakyas")
            .ok_or_else(|| CliError::Other("Could not determine config directory".to_string()))?;
        Ok(dirs.config_dir().to_path_buf())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: DEFAULT_API_URL.to_string(),
            ping_url: DEFAULT_PING_URL.to_string(),
            active_org_id: None,
            active_org_name: None,
            active_org_timezone: None,
            active_project_id: None,
            active_project_name: None,
            format: "table".to_string(),
            color: true,
        }
    }
}

/// Runtime context that combines config, credentials, and CLI overrides
#[derive(Debug)]
pub struct Context {
    pub config: Config,
    org_override: Option<String>,
    project_override: Option<String>,
    format_override: Option<OutputFormat>,
}

// Path-injectable methods for testing
impl Config {
    /// Load config from a specific path
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(CliError::ConfigRead)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to a specific path
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), CliError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }
}

impl Context {
    pub fn load() -> Result<Self> {
        let config = Config::load()?;
        Ok(Self {
            config,
            org_override: None,
            project_override: None,
            format_override: None,
        })
    }

    /// Create context with a specific config (for testing)
    pub fn with_config(config: Config) -> Self {
        Self {
            config,
            org_override: None,
            project_override: None,
            format_override: None,
        }
    }

    pub fn override_org(&mut self, org: String) {
        self.org_override = Some(org);
    }

    pub fn override_project(&mut self, project: String) {
        self.project_override = Some(project);
    }

    pub fn set_format(&mut self, format: OutputFormat) {
        self.format_override = Some(format);
    }

    pub fn api_url(&self) -> String {
        std::env::var("API_URL").unwrap_or_else(|_| self.config.api_url.clone())
    }

    pub fn ping_url(&self) -> String {
        std::env::var("PING_URL").unwrap_or_else(|_| self.config.ping_url.clone())
    }

    /// Get the web app URL (derived from API URL).
    /// Used for dashboard links.
    pub fn app_url(&self) -> String {
        // Default: same base as API URL but without /api path
        // e.g., https://api.pakyas.com -> https://app.pakyas.com
        let api_url = self.api_url();
        api_url.replace("//api.", "//app.").replace("/api", "")
    }

    pub fn active_org_id(&self) -> Option<&str> {
        self.org_override
            .as_deref()
            .or(self.config.active_org_id.as_deref())
    }

    pub fn active_org_name(&self) -> Option<&str> {
        self.config.active_org_name.as_deref()
    }

    pub fn active_project_id(&self) -> Option<&str> {
        self.project_override
            .as_deref()
            .or(self.config.active_project_id.as_deref())
    }

    pub fn active_project_name(&self) -> Option<&str> {
        self.config.active_project_name.as_deref()
    }

    pub fn output_format(&self) -> OutputFormat {
        self.format_override.unwrap_or_default()
    }

    pub fn require_org(&self) -> Result<&str, CliError> {
        self.active_org_id().ok_or(CliError::NoOrgSelected)
    }

    pub fn require_project(&self) -> Result<&str, CliError> {
        self.active_project_id().ok_or(CliError::NoProjectSelected)
    }

    pub fn save_config(&self) -> Result<(), CliError> {
        self.config.save()
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    // Helper to temporarily remove env var during test
    struct EnvVarGuard {
        name: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn new(name: &'static str) -> Self {
            let original = std::env::var(name).ok();
            // SAFETY: Tests run serially via #[serial] attribute
            unsafe { std::env::remove_var(name) };
            Self { name, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(ref val) = self.original {
                // SAFETY: Tests run serially via #[serial] attribute
                unsafe { std::env::set_var(self.name, val) };
            }
        }
    }

    // ============== Config Tests ==============

    #[test]
    fn test_config_default_values() {
        let config = Config::default();

        assert_eq!(config.api_url, DEFAULT_API_URL);
        assert_eq!(config.ping_url, DEFAULT_PING_URL);
        assert!(config.active_org_id.is_none());
        assert!(config.active_org_name.is_none());
        assert!(config.active_project_id.is_none());
        assert!(config.active_project_name.is_none());
        assert_eq!(config.format, "table");
        assert!(config.color);
    }

    #[test]
    fn test_config_default_api_url() {
        let config = Config::default();
        assert_eq!(config.api_url, DEFAULT_API_URL);
    }

    #[test]
    fn test_config_default_ping_url() {
        let config = Config::default();
        assert_eq!(config.ping_url, DEFAULT_PING_URL);
    }

    #[test]
    fn test_config_serialize_deserialize_roundtrip() {
        let config = Config {
            api_url: "https://custom.api.com".to_string(),
            ping_url: "https://custom.ping.com".to_string(),
            active_org_id: Some("org-123".to_string()),
            active_org_name: Some("My Org".to_string()),
            active_org_timezone: Some("Asia/Manila".to_string()),
            active_project_id: Some("proj-456".to_string()),
            active_project_name: Some("My Project".to_string()),
            format: "json".to_string(),
            color: false,
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.api_url, config.api_url);
        assert_eq!(parsed.ping_url, config.ping_url);
        assert_eq!(parsed.active_org_id, config.active_org_id);
        assert_eq!(parsed.active_org_name, config.active_org_name);
        assert_eq!(parsed.active_org_timezone, config.active_org_timezone);
        assert_eq!(parsed.active_project_id, config.active_project_id);
        assert_eq!(parsed.active_project_name, config.active_project_name);
        assert_eq!(parsed.format, config.format);
        assert_eq!(parsed.color, config.color);
    }

    #[test]
    fn test_config_load_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let config = Config::load_from_path(&path).unwrap();

        assert_eq!(config.api_url, DEFAULT_API_URL);
        assert_eq!(config.ping_url, DEFAULT_PING_URL);
    }

    #[test]
    fn test_config_load_valid_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        let content = r#"
api_url = "https://test.api.com"
ping_url = "https://test.ping.com"
active_org_id = "org-test"
format = "json"
color = false
"#;
        std::fs::write(&path, content).unwrap();

        let config = Config::load_from_path(&path).unwrap();

        assert_eq!(config.api_url, "https://test.api.com");
        assert_eq!(config.ping_url, "https://test.ping.com");
        assert_eq!(config.active_org_id, Some("org-test".to_string()));
        assert_eq!(config.format, "json");
        assert!(!config.color);
    }

    #[test]
    fn test_config_load_partial_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        // Only set api_url, other fields should use defaults
        std::fs::write(&path, r#"api_url = "https://custom.api.com""#).unwrap();

        let config = Config::load_from_path(&path).unwrap();

        assert_eq!(config.api_url, "https://custom.api.com");
        assert_eq!(config.ping_url, DEFAULT_PING_URL); // default
        assert!(config.color); // default true
        assert!(config.active_org_id.is_none()); // default None
    }

    #[test]
    fn test_config_load_invalid_toml() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, "not valid toml [[[").unwrap();

        let result = Config::load_from_path(&path);

        assert!(result.is_err());
        assert!(matches!(result, Err(CliError::ConfigParse(_))));
    }

    #[test]
    fn test_config_save_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir
            .path()
            .join("nested")
            .join("dir")
            .join("config.toml");

        let config = Config::default();
        config.save_to_path(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn test_config_save_writes_valid_toml() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("config.toml");

        let config = Config {
            api_url: "https://saved.api.com".to_string(),
            ping_url: "https://saved.ping.com".to_string(),
            active_org_id: Some("saved-org".to_string()),
            active_org_name: None,
            active_org_timezone: Some("America/New_York".to_string()),
            active_project_id: None,
            active_project_name: None,
            format: "table".to_string(),
            color: true,
        };

        config.save_to_path(&path).unwrap();

        let loaded = Config::load_from_path(&path).unwrap();

        assert_eq!(loaded.api_url, "https://saved.api.com");
        assert_eq!(loaded.ping_url, "https://saved.ping.com");
        assert_eq!(loaded.active_org_id, Some("saved-org".to_string()));
        assert_eq!(
            loaded.active_org_timezone,
            Some("America/New_York".to_string())
        );
    }

    // ============== Context Tests ==============

    #[test]
    #[serial]
    fn test_context_api_url_from_config() {
        let _guard = EnvVarGuard::new("API_URL");

        let config = Config {
            api_url: "https://config.api.com".to_string(),
            ..Config::default()
        };
        let ctx = Context::with_config(config);

        assert_eq!(ctx.api_url(), "https://config.api.com");
    }

    #[test]
    #[serial]
    fn test_context_api_url_from_env() {
        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::set_var("API_URL", "https://env.api.com") };

        let config = Config {
            api_url: "https://config.api.com".to_string(),
            ..Config::default()
        };
        let ctx = Context::with_config(config);

        assert_eq!(ctx.api_url(), "https://env.api.com");

        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::remove_var("API_URL") };
    }

    #[test]
    #[serial]
    fn test_context_ping_url_from_env() {
        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::set_var("PING_URL", "https://env.ping.com") };

        let config = Config {
            ping_url: "https://config.ping.com".to_string(),
            ..Config::default()
        };
        let ctx = Context::with_config(config);

        assert_eq!(ctx.ping_url(), "https://env.ping.com");

        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::remove_var("PING_URL") };
    }

    #[test]
    fn test_context_org_override() {
        let mut config = Config::default();
        config.active_org_id = Some("config-org".to_string());

        let mut ctx = Context::with_config(config);

        // Without override, returns config value
        assert_eq!(ctx.active_org_id(), Some("config-org"));

        // With override, returns override
        ctx.override_org("override-org".to_string());
        assert_eq!(ctx.active_org_id(), Some("override-org"));
    }

    #[test]
    fn test_context_project_override() {
        let mut config = Config::default();
        config.active_project_id = Some("config-project".to_string());

        let mut ctx = Context::with_config(config);

        // Without override, returns config value
        assert_eq!(ctx.active_project_id(), Some("config-project"));

        // With override, returns override
        ctx.override_project("override-project".to_string());
        assert_eq!(ctx.active_project_id(), Some("override-project"));
    }

    #[test]
    fn test_context_require_org_error() {
        let config = Config::default();
        let ctx = Context::with_config(config);

        let result = ctx.require_org();

        assert!(matches!(result, Err(CliError::NoOrgSelected)));
    }

    #[test]
    fn test_context_require_project_error() {
        let config = Config::default();
        let ctx = Context::with_config(config);

        let result = ctx.require_project();

        assert!(matches!(result, Err(CliError::NoProjectSelected)));
    }

    #[test]
    fn test_context_active_org_fallback() {
        let mut config = Config::default();
        config.active_org_id = Some("fallback-org".to_string());

        let ctx = Context::with_config(config);

        // No override set, should fall back to config
        assert_eq!(ctx.active_org_id(), Some("fallback-org"));
    }
}
