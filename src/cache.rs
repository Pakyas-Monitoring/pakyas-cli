use crate::config::Config;
use crate::error::CliError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

const CACHE_TTL_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub public_id: Uuid,
    pub check_id: Uuid,
    pub name: String,
    pub cached_at: DateTime<Utc>,
}

impl CacheEntry {
    pub fn is_stale(&self) -> bool {
        let age = Utc::now() - self.cached_at;
        age > Duration::hours(CACHE_TTL_HOURS)
    }
}

/// Cache structure: project_id -> slug -> CacheEntry
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CheckCache {
    projects: HashMap<String, HashMap<String, CacheEntry>>,
}

impl CheckCache {
    /// Load cache from disk
    pub fn load() -> Result<Self, CliError> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path).map_err(CliError::ConfigRead)?;
        let cache: CheckCache = serde_json::from_str(&content)?;
        Ok(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<(), CliError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    /// Get cache file path
    fn path() -> Result<PathBuf, CliError> {
        let config_dir = Config::config_dir()?;
        Ok(config_dir.join("cache").join("checks.json"))
    }

    /// Load cache from a specific path
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, CliError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path).map_err(CliError::ConfigRead)?;
        let cache: CheckCache = serde_json::from_str(&content)?;
        Ok(cache)
    }

    /// Save cache to a specific path
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), CliError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content).map_err(CliError::ConfigWrite)?;
        Ok(())
    }

    /// Look up a check's public_id by slug (cache-first)
    /// Returns None if not in cache or stale
    pub fn get(&self, project_id: &str, slug: &str) -> Option<&CacheEntry> {
        self.projects
            .get(project_id)
            .and_then(|project| project.get(slug))
            .filter(|entry| !entry.is_stale())
    }

    /// Get a check's entry by check_id
    pub fn get_by_check_id(&self, project_id: &str, check_id: &Uuid) -> Option<&CacheEntry> {
        self.projects.get(project_id).and_then(|project| {
            project
                .values()
                .find(|entry| &entry.check_id == check_id && !entry.is_stale())
        })
    }

    /// Update cache with a single check
    pub fn set(
        &mut self,
        project_id: &str,
        slug: &str,
        check_id: Uuid,
        public_id: Uuid,
        name: String,
    ) {
        let project = self.projects.entry(project_id.to_string()).or_default();
        project.insert(
            slug.to_string(),
            CacheEntry {
                public_id,
                check_id,
                name,
                cached_at: Utc::now(),
            },
        );
    }

    /// Bulk update cache from a list of checks
    pub fn update_from_checks<I, C>(&mut self, project_id: &str, checks: I)
    where
        I: IntoIterator<Item = C>,
        C: CheckLike,
    {
        let project = self.projects.entry(project_id.to_string()).or_default();

        // Clear stale entries first
        project.retain(|_, entry| !entry.is_stale());

        // Add/update entries
        for check in checks {
            project.insert(
                check.slug().to_string(),
                CacheEntry {
                    public_id: check.public_id(),
                    check_id: check.id(),
                    name: check.name().to_string(),
                    cached_at: Utc::now(),
                },
            );
        }
    }

    /// Remove a single entry from cache
    pub fn invalidate(&mut self, project_id: &str, slug: &str) {
        if let Some(project) = self.projects.get_mut(project_id) {
            project.remove(slug);
        }
    }

    /// Clear all entries for a project
    pub fn clear_project(&mut self, project_id: &str) {
        self.projects.remove(project_id);
    }
}

/// Trait for types that can be cached (Check API response)
pub trait CheckLike {
    fn id(&self) -> Uuid;
    fn public_id(&self) -> Uuid;
    fn slug(&self) -> &str;
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper struct implementing CheckLike for testing
    struct MockCheck {
        id: Uuid,
        public_id: Uuid,
        slug: String,
        name: String,
    }

    impl CheckLike for MockCheck {
        fn id(&self) -> Uuid {
            self.id
        }
        fn public_id(&self) -> Uuid {
            self.public_id
        }
        fn slug(&self) -> &str {
            &self.slug
        }
        fn name(&self) -> &str {
            &self.name
        }
    }

    // ============== Existing Tests ==============

    #[test]
    fn test_cache_entry_stale() {
        let fresh = CacheEntry {
            public_id: Uuid::new_v4(),
            check_id: Uuid::new_v4(),
            name: "test".to_string(),
            cached_at: Utc::now(),
        };
        assert!(!fresh.is_stale());

        let stale = CacheEntry {
            public_id: Uuid::new_v4(),
            check_id: Uuid::new_v4(),
            name: "test".to_string(),
            cached_at: Utc::now() - Duration::hours(25),
        };
        assert!(stale.is_stale());
    }

    #[test]
    fn test_cache_get_set() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";
        let slug = "my-check";
        let check_id = Uuid::new_v4();
        let public_id = Uuid::new_v4();

        cache.set(
            project_id,
            slug,
            check_id,
            public_id,
            "My Check".to_string(),
        );

        let entry = cache.get(project_id, slug).unwrap();
        assert_eq!(entry.public_id, public_id);
        assert_eq!(entry.check_id, check_id);
    }

    #[test]
    fn test_cache_invalidate() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";
        let slug = "my-check";

        cache.set(
            project_id,
            slug,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "test".to_string(),
        );
        assert!(cache.get(project_id, slug).is_some());

        cache.invalidate(project_id, slug);
        assert!(cache.get(project_id, slug).is_none());
    }

    // ============== New Tests ==============

    #[test]
    fn test_cache_get_returns_none_for_unknown_project() {
        let cache = CheckCache::default();
        assert!(cache.get("unknown-project", "any-slug").is_none());
    }

    #[test]
    fn test_cache_get_returns_none_for_unknown_slug() {
        let mut cache = CheckCache::default();
        cache.set(
            "proj-123",
            "known-slug",
            Uuid::new_v4(),
            Uuid::new_v4(),
            "test".to_string(),
        );

        assert!(cache.get("proj-123", "unknown-slug").is_none());
    }

    #[test]
    fn test_cache_get_filters_stale_entries() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";
        let slug = "stale-check";

        // Manually insert a stale entry
        let stale_entry = CacheEntry {
            public_id: Uuid::new_v4(),
            check_id: Uuid::new_v4(),
            name: "Stale Check".to_string(),
            cached_at: Utc::now() - Duration::hours(25), // 25 hours ago = stale
        };

        cache.projects.insert(project_id.to_string(), {
            let mut m = HashMap::new();
            m.insert(slug.to_string(), stale_entry);
            m
        });

        // get() should return None for stale entry
        assert!(cache.get(project_id, slug).is_none());
    }

    #[test]
    fn test_cache_get_by_check_id() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";
        let check_id = Uuid::new_v4();
        let public_id = Uuid::new_v4();

        cache.set(
            project_id,
            "my-slug",
            check_id,
            public_id,
            "Test".to_string(),
        );

        let entry = cache.get_by_check_id(project_id, &check_id).unwrap();
        assert_eq!(entry.public_id, public_id);
        assert_eq!(entry.check_id, check_id);
    }

    #[test]
    fn test_cache_get_by_check_id_filters_stale() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";
        let check_id = Uuid::new_v4();

        // Insert a stale entry
        let stale_entry = CacheEntry {
            public_id: Uuid::new_v4(),
            check_id,
            name: "Stale".to_string(),
            cached_at: Utc::now() - Duration::hours(25),
        };

        cache.projects.insert(project_id.to_string(), {
            let mut m = HashMap::new();
            m.insert("stale-slug".to_string(), stale_entry);
            m
        });

        assert!(cache.get_by_check_id(project_id, &check_id).is_none());
    }

    #[test]
    fn test_cache_update_from_checks() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";

        let checks = vec![
            MockCheck {
                id: Uuid::new_v4(),
                public_id: Uuid::new_v4(),
                slug: "check-1".to_string(),
                name: "Check One".to_string(),
            },
            MockCheck {
                id: Uuid::new_v4(),
                public_id: Uuid::new_v4(),
                slug: "check-2".to_string(),
                name: "Check Two".to_string(),
            },
        ];

        cache.update_from_checks(project_id, checks);

        assert!(cache.get(project_id, "check-1").is_some());
        assert!(cache.get(project_id, "check-2").is_some());
    }

    #[test]
    fn test_cache_update_from_checks_clears_stale() {
        let mut cache = CheckCache::default();
        let project_id = "proj-123";

        // Insert a stale entry
        let stale_entry = CacheEntry {
            public_id: Uuid::new_v4(),
            check_id: Uuid::new_v4(),
            name: "Stale".to_string(),
            cached_at: Utc::now() - Duration::hours(25),
        };

        cache.projects.insert(project_id.to_string(), {
            let mut m = HashMap::new();
            m.insert("stale-check".to_string(), stale_entry);
            m
        });

        // Update with new checks
        let checks = vec![MockCheck {
            id: Uuid::new_v4(),
            public_id: Uuid::new_v4(),
            slug: "new-check".to_string(),
            name: "New Check".to_string(),
        }];

        cache.update_from_checks(project_id, checks);

        // Stale entry should be removed
        let project = cache.projects.get(project_id).unwrap();
        assert!(!project.contains_key("stale-check"));
        assert!(project.contains_key("new-check"));
    }

    #[test]
    fn test_cache_clear_project() {
        let mut cache = CheckCache::default();

        cache.set(
            "proj-a",
            "check-1",
            Uuid::new_v4(),
            Uuid::new_v4(),
            "A1".to_string(),
        );
        cache.set(
            "proj-a",
            "check-2",
            Uuid::new_v4(),
            Uuid::new_v4(),
            "A2".to_string(),
        );
        cache.set(
            "proj-b",
            "check-1",
            Uuid::new_v4(),
            Uuid::new_v4(),
            "B1".to_string(),
        );

        cache.clear_project("proj-a");

        assert!(cache.get("proj-a", "check-1").is_none());
        assert!(cache.get("proj-a", "check-2").is_none());
        assert!(cache.get("proj-b", "check-1").is_some());
    }

    #[test]
    fn test_cache_multiple_projects_isolation() {
        let mut cache = CheckCache::default();

        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        cache.set("proj-a", "check", Uuid::new_v4(), id_a, "A".to_string());
        cache.set("proj-b", "check", Uuid::new_v4(), id_b, "B".to_string());

        let entry_a = cache.get("proj-a", "check").unwrap();
        let entry_b = cache.get("proj-b", "check").unwrap();

        assert_eq!(entry_a.name, "A");
        assert_eq!(entry_a.public_id, id_a);
        assert_eq!(entry_b.name, "B");
        assert_eq!(entry_b.public_id, id_b);
    }

    #[test]
    fn test_cache_load_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        let cache = CheckCache::load_from_path(&path).unwrap();

        assert!(cache.projects.is_empty());
    }

    #[test]
    fn test_cache_save_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("cache.json");

        let mut cache = CheckCache::default();
        let check_id = Uuid::new_v4();
        let public_id = Uuid::new_v4();
        cache.set(
            "proj-123",
            "my-check",
            check_id,
            public_id,
            "My Check".to_string(),
        );

        cache.save_to_path(&path).unwrap();

        let loaded = CheckCache::load_from_path(&path).unwrap();
        let entry = loaded.get("proj-123", "my-check").unwrap();

        assert_eq!(entry.check_id, check_id);
        assert_eq!(entry.public_id, public_id);
        assert_eq!(entry.name, "My Check");
    }

    #[test]
    fn test_cache_load_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("cache.json");

        std::fs::write(&path, "not valid json {{{").unwrap();

        let result = CheckCache::load_from_path(&path);

        assert!(result.is_err());
        assert!(matches!(result, Err(CliError::Json(_))));
    }
}
