//! Global lock and atomic write utilities for safe config/credentials updates.
//!
//! This module provides:
//! - A global file lock to prevent concurrent writes from multiple CLI processes
//! - Atomic write operations using temp files and rename

use crate::config::Config;
use crate::error::CliError;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// A guard that holds an exclusive lock on the pakyas config directory.
/// The lock is released when this guard is dropped.
pub struct GlobalLock {
    _lock_file: File,
}

impl GlobalLock {
    /// Acquire an exclusive lock on the pakyas config directory.
    ///
    /// This prevents other CLI processes from modifying config or credentials
    /// while we hold the lock. The lock is released when this guard is dropped.
    ///
    /// # Errors
    /// Returns `CliError::LockFailed` if the lock cannot be acquired.
    pub fn acquire() -> Result<Self, CliError> {
        let lock_path = Config::config_dir()?.join("pakyas.lock");

        // Ensure parent directory exists
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }

        // Don't truncate - we just need the file to exist for locking
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(CliError::ConfigWrite)?;

        lock_file.lock_exclusive().map_err(|_| CliError::LockFailed)?;

        Ok(Self {
            _lock_file: lock_file,
        })
    }

    /// Acquire a lock with a custom path (for testing).
    #[cfg(test)]
    pub fn acquire_at_path(lock_path: &Path) -> Result<Self, CliError> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(CliError::ConfigWrite)?;
        }

        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(CliError::ConfigWrite)?;

        lock_file.lock_exclusive().map_err(|_| CliError::LockFailed)?;

        Ok(Self {
            _lock_file: lock_file,
        })
    }
}

/// Write content to a file atomically using a temporary file and rename.
///
/// This ensures that the file is never left in a partially-written state,
/// even if the process is killed during the write.
///
/// # Arguments
/// * `path` - The target file path
/// * `content` - The content to write
///
/// # Errors
/// Returns `CliError::ConfigWrite` if the write fails.
pub fn atomic_write(path: &Path, content: &str) -> Result<(), CliError> {
    let dir = path
        .parent()
        .ok_or_else(|| CliError::ConfigWrite(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Path has no parent directory",
        )))?;

    // Ensure parent directory exists
    std::fs::create_dir_all(dir).map_err(CliError::ConfigWrite)?;

    // Create temp file in same directory (required for atomic rename on same filesystem)
    let mut temp_file = NamedTempFile::new_in(dir).map_err(CliError::ConfigWrite)?;
    temp_file
        .write_all(content.as_bytes())
        .map_err(CliError::ConfigWrite)?;
    temp_file.flush().map_err(CliError::ConfigWrite)?;

    // persist() handles atomic rename cross-platform (removes existing on Windows)
    temp_file
        .persist(path)
        .map_err(|e| CliError::ConfigWrite(e.error))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        atomic_write(&path, "hello world").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        std::fs::write(&path, "original").unwrap();
        atomic_write(&path, "replaced").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "replaced");
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nested").join("dir").join("test.txt");

        atomic_write(&path, "nested content").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "nested content");
    }

    #[test]
    fn test_global_lock_acquire_release() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("test.lock");

        // Acquire lock
        let lock = GlobalLock::acquire_at_path(&lock_path).unwrap();
        assert!(lock_path.exists());

        // Drop the lock
        drop(lock);

        // Should be able to acquire again
        let _lock2 = GlobalLock::acquire_at_path(&lock_path).unwrap();
    }

    #[test]
    fn test_atomic_write_preserves_unicode() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("unicode.txt");

        let content = "Hello, \u{4e16}\u{754c}! \u{1f680}"; // "Hello, World! Rocket"
        atomic_write(&path, content).unwrap();

        let read_content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_content, content);
    }
}
