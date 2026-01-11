use crate::config::Config;
use crate::error::CliError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub api_key: Option<String>,
    pub user_email: Option<String>,
    pub user_id: Option<String>,
}

impl Credentials {
    pub fn load() -> Result<Self, CliError> {
        // Check environment variable first
        if let Ok(key) = std::env::var("PAKYAS_API_KEY") {
            return Ok(Self {
                api_key: Some(key),
                user_email: None,
                user_id: None,
            });
        }

        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path).map_err(CliError::ConfigRead)?;
        let creds: Credentials = serde_json::from_str(&content)?;
        Ok(creds)
    }

    pub fn save(&self) -> Result<(), CliError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    pub fn clear() -> Result<(), CliError> {
        let path = Self::path()?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(CliError::ConfigWrite)?;
        }
        Ok(())
    }

    pub fn path() -> Result<PathBuf, CliError> {
        let config_dir = Config::config_dir()?;
        Ok(config_dir.join("credentials.json"))
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub fn is_authenticated(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn require_api_key(&self) -> Result<&str, CliError> {
        self.api_key().ok_or(CliError::NotAuthenticated)
    }
}

/// Validate that an API key has the correct format
pub fn validate_api_key(key: &str) -> Result<(), CliError> {
    if !key.starts_with("pk_") {
        return Err(CliError::InvalidApiKey);
    }
    if key.len() < 20 {
        return Err(CliError::InvalidApiKey);
    }
    Ok(())
}

// Path-injectable methods for testing
impl Credentials {
    /// Load credentials from a specific path (env var still takes precedence)
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        // Check environment variable first
        if let Ok(key) = std::env::var("PAKYAS_API_KEY") {
            return Ok(Self {
                api_key: Some(key),
                user_email: None,
                user_id: None,
            });
        }

        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(CliError::ConfigRead)?;
        let creds: Credentials = serde_json::from_str(&content)?;
        Ok(creds)
    }

    /// Save credentials to a specific path
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), CliError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    /// Clear credentials at a specific path
    pub fn clear_at_path(path: &std::path::Path) -> Result<(), CliError> {
        if path.exists() {
            std::fs::remove_file(path).map_err(CliError::ConfigWrite)?;
        }
        Ok(())
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

    #[test]
    fn test_credentials_default() {
        let creds = Credentials::default();
        assert!(creds.api_key.is_none());
        assert!(creds.user_email.is_none());
        assert!(creds.user_id.is_none());
    }

    #[test]
    fn test_credentials_is_authenticated() {
        let creds = Credentials {
            api_key: Some("pk_test_1234567890123456".to_string()),
            user_email: None,
            user_id: None,
        };
        assert!(creds.is_authenticated());
    }

    #[test]
    fn test_credentials_is_not_authenticated() {
        let creds = Credentials::default();
        assert!(!creds.is_authenticated());
    }

    #[test]
    fn test_credentials_api_key_getter() {
        let creds = Credentials {
            api_key: Some("pk_test_key_abc".to_string()),
            user_email: None,
            user_id: None,
        };
        assert_eq!(creds.api_key(), Some("pk_test_key_abc"));

        let empty_creds = Credentials::default();
        assert_eq!(empty_creds.api_key(), None);
    }

    #[test]
    fn test_credentials_require_api_key_success() {
        let creds = Credentials {
            api_key: Some("pk_test_key_12345".to_string()),
            user_email: None,
            user_id: None,
        };
        let result = creds.require_api_key();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "pk_test_key_12345");
    }

    #[test]
    fn test_credentials_require_api_key_error() {
        let creds = Credentials::default();
        let result = creds.require_api_key();
        assert!(matches!(result, Err(CliError::NotAuthenticated)));
    }

    #[test]
    fn test_validate_api_key_valid() {
        // Valid: starts with pk_, at least 20 chars
        assert!(validate_api_key("pk_test_1234567890123456").is_ok());
        assert!(validate_api_key("pk_live_abcdefghijklmnop").is_ok());
        assert!(validate_api_key("pk_12345678901234567890").is_ok());
    }

    #[test]
    fn test_validate_api_key_invalid_prefix() {
        assert!(matches!(
            validate_api_key("sk_test_1234567890123456"),
            Err(CliError::InvalidApiKey)
        ));
        assert!(matches!(
            validate_api_key("invalid_key_here_long"),
            Err(CliError::InvalidApiKey)
        ));
        assert!(matches!(
            validate_api_key("apikey_1234567890123456"),
            Err(CliError::InvalidApiKey)
        ));
    }

    #[test]
    fn test_validate_api_key_too_short() {
        assert!(matches!(
            validate_api_key("pk_short"),
            Err(CliError::InvalidApiKey)
        ));
        assert!(matches!(
            validate_api_key("pk_1234567890123"), // 18 chars
            Err(CliError::InvalidApiKey)
        ));
        assert!(matches!(
            validate_api_key("pk_"),
            Err(CliError::InvalidApiKey)
        ));
    }

    #[test]
    #[serial]
    fn test_credentials_load_from_env_var() {
        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::set_var("PAKYAS_API_KEY", "pk_env_1234567890123456") };

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        // Even with a file present, env var takes precedence
        let file_creds = Credentials {
            api_key: Some("pk_file_key_1234567890".to_string()),
            user_email: Some("file@test.com".to_string()),
            user_id: None,
        };
        file_creds.save_to_path(&path).unwrap();

        let loaded = Credentials::load_from_path(&path).unwrap();

        // Should use env var, not file
        assert_eq!(loaded.api_key, Some("pk_env_1234567890123456".to_string()));
        // Email is None because env var path doesn't load from file
        assert!(loaded.user_email.is_none());

        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::remove_var("PAKYAS_API_KEY") };
    }

    #[test]
    #[serial]
    fn test_credentials_load_missing_file() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let creds = Credentials::load_from_path(&path).unwrap();

        assert!(!creds.is_authenticated());
        assert!(creds.api_key.is_none());
    }

    #[test]
    #[serial]
    fn test_credentials_save_load_roundtrip() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        let creds = Credentials {
            api_key: Some("pk_test_1234567890123456".to_string()),
            user_email: Some("test@example.com".to_string()),
            user_id: Some("user-123".to_string()),
        };

        creds.save_to_path(&path).unwrap();

        let loaded = Credentials::load_from_path(&path).unwrap();

        assert_eq!(loaded.api_key, creds.api_key);
        assert_eq!(loaded.user_email, creds.user_email);
        assert_eq!(loaded.user_id, creds.user_id);
    }

    #[test]
    fn test_credentials_clear() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        // Create a file first
        std::fs::write(&path, r#"{"api_key": "pk_test_key_12345678"}"#).unwrap();
        assert!(path.exists());

        Credentials::clear_at_path(&path).unwrap();

        assert!(!path.exists());
    }

    #[test]
    #[serial]
    fn test_credentials_load_invalid_json() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        std::fs::write(&path, "not valid json {{{").unwrap();

        let result = Credentials::load_from_path(&path);

        assert!(result.is_err());
        assert!(matches!(result, Err(CliError::Json(_))));
    }
}
