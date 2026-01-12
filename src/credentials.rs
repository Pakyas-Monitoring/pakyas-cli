//! Credentials management for the Pakyas CLI.
//!
//! This module handles API key storage with per-org support (V2 schema) and
//! backward-compatible migration from the legacy single-key format (V1).
//!
//! # Schema Versions
//!
//! ## V1 (Legacy)
//! ```json
//! {
//!   "api_key": "pk_...",
//!   "user_email": "user@example.com",
//!   "user_id": "user-123"
//! }
//! ```
//!
//! ## V2 (Per-org)
//! ```json
//! {
//!   "version": 2,
//!   "orgs": {
//!     "org_abc123": {
//!       "api_key": "pk_...",
//!       "key_id": "key-uuid",
//!       "label": "Moon-Macbook-2026-01-12",
//!       "added_at": "2026-01-12T10:30:00Z",
//!       "last_verified": "2026-01-12T10:35:00Z"
//!     }
//!   },
//!   "legacy_api_key": "pk_..."
//! }
//! ```

use crate::config::Config;
use crate::error::CliError;
use crate::lock::atomic_write;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// V2 Schema Types
// ============================================================================

/// Credential for a single organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgCredential {
    /// The API key for this organization.
    pub api_key: String,

    /// The server-side key ID (for key management).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,

    /// Human-readable label (e.g., "Moon-Macbook-2026-01-12").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// When this credential was added.
    pub added_at: DateTime<Utc>,

    /// When this credential was last verified with the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified: Option<DateTime<Utc>>,
}

impl OrgCredential {
    /// Create a new org credential with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            key_id: None,
            label: None,
            added_at: Utc::now(),
            last_verified: None,
        }
    }

    /// Create a new org credential with all fields.
    pub fn with_details(api_key: String, key_id: Option<String>, label: Option<String>) -> Self {
        Self {
            api_key,
            key_id,
            label,
            added_at: Utc::now(),
            last_verified: Some(Utc::now()),
        }
    }
}

/// V2 credentials with per-org API key storage.
#[derive(Debug, Serialize, Deserialize)]
pub struct CredentialsV2 {
    /// Schema version (always 2).
    pub version: u8,

    /// Per-org API keys, keyed by org ID.
    #[serde(default)]
    pub orgs: HashMap<String, OrgCredential>,

    /// Legacy API key from V1 migration (not associated with any org yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_api_key: Option<String>,
}

impl Default for CredentialsV2 {
    fn default() -> Self {
        Self {
            version: 2,
            orgs: HashMap::new(),
            legacy_api_key: None,
        }
    }
}

impl CredentialsV2 {
    /// Load credentials from the default path, with V1 migration if needed.
    ///
    /// If `PAKYAS_API_KEY` env var is set, returns credentials with that key
    /// as the legacy key (for backward compatibility).
    pub fn load() -> Result<Self, CliError> {
        Self::load_from_path(&Self::path()?)
    }

    /// Load credentials from a specific path (for testing).
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        // Check environment variable first
        if let Ok(key) = std::env::var("PAKYAS_API_KEY") {
            return Ok(Self {
                version: 2,
                orgs: HashMap::new(),
                legacy_api_key: Some(key),
            });
        }

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path).map_err(CliError::ConfigRead)?;

        // Try to parse as V2 first
        if let Ok(v2) = serde_json::from_str::<CredentialsV2>(&content) {
            if v2.version == 2 {
                return Ok(v2);
            }
        }

        // Try to parse as V1 and migrate
        match serde_json::from_str::<CredentialsV1>(&content) {
            Ok(v1) => Ok(Self::migrate_from_v1(v1)),
            Err(_) => Err(CliError::CredentialsCorrupted),
        }
    }

    /// Save credentials to the default path atomically.
    pub fn save(&self) -> Result<(), CliError> {
        let path = Self::path()?;
        self.save_to_path(&path)
    }

    /// Save credentials to a specific path atomically.
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), CliError> {
        let content = serde_json::to_string_pretty(self)?;
        atomic_write(path, &content)
    }

    /// Clear all credentials at the default path.
    pub fn clear() -> Result<(), CliError> {
        let path = Self::path()?;
        Self::clear_at_path(&path)
    }

    /// Clear credentials at a specific path.
    pub fn clear_at_path(path: &std::path::Path) -> Result<(), CliError> {
        if path.exists() {
            std::fs::remove_file(path).map_err(CliError::ConfigWrite)?;
        }
        Ok(())
    }

    /// Get the default credentials file path.
    pub fn path() -> Result<PathBuf, CliError> {
        let config_dir = Config::config_dir()?;
        Ok(config_dir.join("credentials.json"))
    }

    // ========================================================================
    // Per-org key management
    // ========================================================================

    /// Get the credential for a specific organization.
    pub fn get_for_org(&self, org_id: &str) -> Option<&OrgCredential> {
        self.orgs.get(org_id)
    }

    /// Get mutable credential for a specific organization.
    pub fn get_for_org_mut(&mut self, org_id: &str) -> Option<&mut OrgCredential> {
        self.orgs.get_mut(org_id)
    }

    /// Set the credential for a specific organization.
    pub fn set_for_org(&mut self, org_id: impl Into<String>, cred: OrgCredential) {
        self.orgs.insert(org_id.into(), cred);
    }

    /// Remove the credential for a specific organization.
    pub fn remove_for_org(&mut self, org_id: &str) -> Option<OrgCredential> {
        self.orgs.remove(org_id)
    }

    /// Check if we have a key stored for a specific organization.
    pub fn has_key_for_org(&self, org_id: &str) -> bool {
        self.orgs.contains_key(org_id)
    }

    /// List all org IDs that have stored keys.
    pub fn list_orgs_with_keys(&self) -> Vec<&str> {
        self.orgs.keys().map(|s| s.as_str()).collect()
    }

    /// Remove the legacy key (used after promoting it to an org slot).
    pub fn remove_legacy_key(&mut self) {
        self.legacy_api_key = None;
    }

    /// Check if there's a legacy key that hasn't been associated with an org.
    pub fn has_legacy_key(&self) -> bool {
        self.legacy_api_key.is_some()
    }

    /// Get the legacy API key if present.
    pub fn legacy_key(&self) -> Option<&str> {
        self.legacy_api_key.as_deref()
    }

    /// Promote a legacy key to an org slot if the slot is empty.
    ///
    /// Returns true if promotion happened, false if:
    /// - No legacy key exists
    /// - Org slot already has a key
    pub fn promote_legacy_key_to_org(&mut self, org_id: &str) -> bool {
        if let Some(legacy_key) = self.legacy_api_key.take() {
            if !self.orgs.contains_key(org_id) {
                self.orgs
                    .insert(org_id.to_string(), OrgCredential::new(legacy_key));
                return true;
            } else {
                // Put it back - org already has a key
                self.legacy_api_key = Some(legacy_key);
            }
        }
        false
    }

    // ========================================================================
    // Migration
    // ========================================================================

    /// Migrate from V1 credentials to V2.
    ///
    /// The V1 api_key is moved to legacy_api_key (NOT associated with any org).
    /// It will be promoted to an org slot on the first successful API call.
    fn migrate_from_v1(v1: CredentialsV1) -> Self {
        Self {
            version: 2,
            orgs: HashMap::new(),
            legacy_api_key: v1.api_key,
        }
    }

    /// Check if we have any credentials (for backward compatibility).
    pub fn is_authenticated(&self) -> bool {
        !self.orgs.is_empty() || self.legacy_api_key.is_some()
    }
}

// ============================================================================
// V1 Schema (Legacy) - for migration only
// ============================================================================

#[derive(Debug, Deserialize)]
struct CredentialsV1 {
    api_key: Option<String>,
    #[allow(dead_code)]
    user_email: Option<String>,
    #[allow(dead_code)]
    user_id: Option<String>,
}

// ============================================================================
// Legacy Credentials wrapper (backward compatibility)
// ============================================================================

/// Legacy credentials struct for backward compatibility.
///
/// This is kept for existing code that uses the old API. New code should use
/// `CredentialsV2` directly.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub api_key: Option<String>,
    pub user_email: Option<String>,
    pub user_id: Option<String>,
}

impl Credentials {
    pub fn load() -> Result<Self, CliError> {
        // Load V2 and extract the "best" key for backward compatibility
        // (V2 already handles PAKYAS_API_KEY env var)
        let v2 = CredentialsV2::load()?;

        // For legacy compatibility: prefer legacy_key, then any org key
        let api_key = v2
            .legacy_api_key
            .or_else(|| v2.orgs.values().next().map(|c| c.api_key.clone()));

        Ok(Self {
            api_key,
            user_email: None,
            user_id: None,
        })
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

// Path-injectable methods for testing
impl Credentials {
    /// Load credentials from a specific path (env var still takes precedence via V2)
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        // Load via V2 to get env var handling
        let v2 = CredentialsV2::load_from_path(path)?;

        let api_key = v2
            .legacy_api_key
            .or_else(|| v2.orgs.values().next().map(|c| c.api_key.clone()));

        Ok(Self {
            api_key,
            user_email: None,
            user_id: None,
        })
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

    // ============== Legacy Credentials Tests ==============

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

        // Save legacy format directly (simulating V1)
        let creds = Credentials {
            api_key: Some("pk_test_1234567890123456".to_string()),
            user_email: Some("test@example.com".to_string()),
            user_id: Some("user-123".to_string()),
        };
        creds.save_to_path(&path).unwrap();

        // Loading via load_from_path now goes through V2 migration,
        // which only preserves the api_key (user_email/user_id are legacy fields)
        let loaded = Credentials::load_from_path(&path).unwrap();

        assert_eq!(loaded.api_key, creds.api_key);
        // Note: user_email and user_id are not preserved through V2 migration
        assert!(loaded.user_email.is_none());
        assert!(loaded.user_id.is_none());
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

        // Invalid JSON is treated as corrupted credentials since
        // loading now goes through CredentialsV2
        assert!(result.is_err());
        assert!(matches!(result, Err(CliError::CredentialsCorrupted)));
    }

    // ============== CredentialsV2 Tests ==============

    #[test]
    fn test_credentials_v2_default() {
        let creds = CredentialsV2::default();
        assert_eq!(creds.version, 2);
        assert!(creds.orgs.is_empty());
        assert!(creds.legacy_api_key.is_none());
    }

    #[test]
    #[serial]
    fn test_credentials_v2_load_save_roundtrip() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        let mut creds = CredentialsV2::default();
        creds.set_for_org(
            "org_test123",
            OrgCredential::new("pk_test_1234567890123456".to_string()),
        );
        creds.legacy_api_key = Some("pk_legacy_key_1234567890".to_string());

        creds.save_to_path(&path).unwrap();

        let loaded = CredentialsV2::load_from_path(&path).unwrap();

        assert_eq!(loaded.version, 2);
        assert!(loaded.has_key_for_org("org_test123"));
        assert_eq!(
            loaded.get_for_org("org_test123").unwrap().api_key,
            "pk_test_1234567890123456"
        );
        assert_eq!(
            loaded.legacy_api_key,
            Some("pk_legacy_key_1234567890".to_string())
        );
    }

    #[test]
    #[serial]
    fn test_credentials_v1_to_v2_migration() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        // Write V1 format
        let v1_content = r#"{
            "api_key": "pk_v1_key_1234567890123456",
            "user_email": "user@test.com",
            "user_id": "user-123"
        }"#;
        std::fs::write(&path, v1_content).unwrap();

        // Load as V2 (should migrate)
        let v2 = CredentialsV2::load_from_path(&path).unwrap();

        assert_eq!(v2.version, 2);
        assert!(v2.orgs.is_empty()); // V1 key goes to legacy, not org slot
        assert_eq!(
            v2.legacy_api_key,
            Some("pk_v1_key_1234567890123456".to_string())
        );
    }

    #[test]
    #[serial]
    fn test_credentials_corrupted_json_returns_error() {
        let _guard = EnvVarGuard::new("PAKYAS_API_KEY");

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        // Write corrupted JSON
        std::fs::write(&path, "{{{ not valid json").unwrap();

        let result = CredentialsV2::load_from_path(&path);

        assert!(matches!(result, Err(CliError::CredentialsCorrupted)));
    }

    #[test]
    fn test_credentials_v2_per_org_operations() {
        let mut creds = CredentialsV2::default();

        // Initially empty
        assert!(!creds.has_key_for_org("org_abc"));
        assert!(creds.get_for_org("org_abc").is_none());
        assert!(creds.list_orgs_with_keys().is_empty());

        // Add a key
        creds.set_for_org(
            "org_abc",
            OrgCredential::new("pk_abc_key_12345678901".to_string()),
        );
        assert!(creds.has_key_for_org("org_abc"));
        assert_eq!(
            creds.get_for_org("org_abc").unwrap().api_key,
            "pk_abc_key_12345678901"
        );
        assert_eq!(creds.list_orgs_with_keys(), vec!["org_abc"]);

        // Add another
        creds.set_for_org(
            "org_xyz",
            OrgCredential::new("pk_xyz_key_12345678901".to_string()),
        );
        assert_eq!(creds.list_orgs_with_keys().len(), 2);

        // Remove one
        let removed = creds.remove_for_org("org_abc");
        assert!(removed.is_some());
        assert!(!creds.has_key_for_org("org_abc"));
        assert!(creds.has_key_for_org("org_xyz"));
    }

    #[test]
    fn test_credentials_v2_legacy_key_promotion() {
        let mut creds = CredentialsV2 {
            legacy_api_key: Some("pk_legacy_1234567890123456".to_string()),
            ..Default::default()
        };

        // Should promote to empty slot
        assert!(creds.promote_legacy_key_to_org("org_new"));
        assert!(creds.has_key_for_org("org_new"));
        assert!(creds.legacy_api_key.is_none());
        assert_eq!(
            creds.get_for_org("org_new").unwrap().api_key,
            "pk_legacy_1234567890123456"
        );
    }

    #[test]
    fn test_credentials_v2_legacy_key_no_overwrite() {
        let mut creds = CredentialsV2 {
            legacy_api_key: Some("pk_legacy_1234567890123456".to_string()),
            ..Default::default()
        };
        creds.set_for_org(
            "org_existing",
            OrgCredential::new("pk_existing_key_1234567".to_string()),
        );

        // Should NOT promote if org already has a key
        assert!(!creds.promote_legacy_key_to_org("org_existing"));
        assert_eq!(
            creds.legacy_api_key,
            Some("pk_legacy_1234567890123456".to_string())
        );
        assert_eq!(
            creds.get_for_org("org_existing").unwrap().api_key,
            "pk_existing_key_1234567"
        );
    }

    #[test]
    fn test_credentials_v2_is_authenticated() {
        let mut creds = CredentialsV2::default();
        assert!(!creds.is_authenticated());

        // With legacy key
        creds.legacy_api_key = Some("pk_test_1234567890123456".to_string());
        assert!(creds.is_authenticated());

        // With org key (no legacy)
        creds.legacy_api_key = None;
        creds.set_for_org(
            "org_test",
            OrgCredential::new("pk_org_key_1234567890".to_string()),
        );
        assert!(creds.is_authenticated());
    }

    #[test]
    #[serial]
    fn test_credentials_v2_env_var_takes_precedence() {
        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::set_var("PAKYAS_API_KEY", "pk_env_1234567890123456") };

        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("credentials.json");

        // Write a V2 file with different keys
        let mut file_creds = CredentialsV2::default();
        file_creds.set_for_org(
            "org_file",
            OrgCredential::new("pk_file_key_1234567890".to_string()),
        );
        file_creds.save_to_path(&path).unwrap();

        // Load should return env var as legacy key
        let loaded = CredentialsV2::load_from_path(&path).unwrap();
        assert_eq!(
            loaded.legacy_api_key,
            Some("pk_env_1234567890123456".to_string())
        );
        // Env var path doesn't load file content
        assert!(loaded.orgs.is_empty());

        // SAFETY: Tests run serially via #[serial] attribute
        unsafe { std::env::remove_var("PAKYAS_API_KEY") };
    }

    #[test]
    fn test_org_credential_new() {
        let cred = OrgCredential::new("pk_test_1234567890123456".to_string());
        assert_eq!(cred.api_key, "pk_test_1234567890123456");
        assert!(cred.key_id.is_none());
        assert!(cred.label.is_none());
        assert!(cred.last_verified.is_none());
    }

    #[test]
    fn test_org_credential_with_details() {
        let cred = OrgCredential::with_details(
            "pk_test_1234567890123456".to_string(),
            Some("key-uuid-123".to_string()),
            Some("My-Laptop-2026".to_string()),
        );
        assert_eq!(cred.api_key, "pk_test_1234567890123456");
        assert_eq!(cred.key_id, Some("key-uuid-123".to_string()));
        assert_eq!(cred.label, Some("My-Laptop-2026".to_string()));
        assert!(cred.last_verified.is_some());
    }
}
