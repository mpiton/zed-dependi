//! SQLite persistent cache for package version information

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::registries::VersionInfo;

/// Default TTL for cache entries (1 hour)
const DEFAULT_TTL_SECS: i64 = 3600;

/// SQLite-based persistent cache
pub struct SqliteCache {
    conn: Mutex<Connection>,
    ttl_secs: i64,
}

impl SqliteCache {
    /// Create a new SQLite cache at the default location (~/.cache/dependi/cache.db)
    pub fn new() -> anyhow::Result<Self> {
        let cache_dir = Self::cache_dir()?;
        std::fs::create_dir_all(&cache_dir)?;
        let db_path = cache_dir.join("cache.db");
        Self::with_path(db_path)
    }

    /// Create a new SQLite cache at a custom path
    pub fn with_path(path: PathBuf) -> anyhow::Result<Self> {
        let conn = Connection::open(&path)?;
        let cache = Self {
            conn: Mutex::new(conn),
            ttl_secs: DEFAULT_TTL_SECS,
        };
        cache.init_schema()?;
        cache.cleanup_expired()?;
        Ok(cache)
    }

    /// Create an in-memory cache (for testing)
    #[cfg(test)]
    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let cache = Self {
            conn: Mutex::new(conn),
            ttl_secs: DEFAULT_TTL_SECS,
        };
        cache.init_schema()?;
        Ok(cache)
    }

    /// Get the cache directory
    fn cache_dir() -> anyhow::Result<PathBuf> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        Ok(cache_dir.join("dependi"))
    }

    /// Initialize the database schema
    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS packages (
                key TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                inserted_at INTEGER NOT NULL,
                ttl_secs INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_expiry ON packages(inserted_at, ttl_secs)",
            [],
        )?;
        Ok(())
    }

    /// Get a value from the cache
    pub fn get(&self, key: &str) -> Option<VersionInfo> {
        let conn = self.conn.lock().unwrap();
        let now = current_timestamp();

        let result: Result<(String, i64, i64), _> = conn.query_row(
            "SELECT data, inserted_at, ttl_secs FROM packages WHERE key = ?",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        match result {
            Ok((data, inserted_at, ttl_secs)) => {
                // Check if expired
                if now > inserted_at + ttl_secs {
                    // Entry is expired, remove it
                    let _ = conn.execute("DELETE FROM packages WHERE key = ?", [key]);
                    None
                } else {
                    // Parse JSON data
                    serde_json::from_str(&data).ok()
                }
            }
            Err(_) => None,
        }
    }

    /// Insert a value into the cache
    pub fn insert(&self, key: String, value: VersionInfo) {
        let conn = self.conn.lock().unwrap();
        let now = current_timestamp();
        let data = match serde_json::to_string(&value) {
            Ok(d) => d,
            Err(_) => return,
        };

        let _ = conn.execute(
            "INSERT OR REPLACE INTO packages (key, data, inserted_at, ttl_secs) VALUES (?, ?, ?, ?)",
            params![key, data, now, self.ttl_secs],
        );
    }

    pub fn cleanup_expired(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let now = current_timestamp();
        let rows = conn.execute(
            "DELETE FROM packages WHERE inserted_at + ttl_secs < ?",
            [now],
        )?;
        Ok(rows)
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_version_info() -> VersionInfo {
        VersionInfo {
            latest: Some("1.0.0".to_string()),
            latest_prerelease: None,
            versions: vec!["1.0.0".to_string(), "0.9.0".to_string()],
            description: Some("Test package".to_string()),
            homepage: None,
            repository: None,
            license: Some("MIT".to_string()),
            vulnerabilities: vec![],
            deprecated: false,
            yanked: false,
            yanked_versions: vec![],
            release_dates: Default::default(),
        }
    }

    #[test]
    fn test_insert_and_get() {
        let cache = SqliteCache::in_memory().unwrap();
        let info = create_test_version_info();

        cache.insert("test:package".to_string(), info.clone());
        let retrieved = cache.get("test:package");

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.latest, info.latest);
        assert_eq!(retrieved.versions, info.versions);
    }

    #[test]
    fn test_get_nonexistent() {
        let cache = SqliteCache::in_memory().unwrap();
        let retrieved = cache.get("nonexistent");
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_overwrite() {
        let cache = SqliteCache::in_memory().unwrap();

        let info1 = VersionInfo {
            latest: Some("1.0.0".to_string()),
            ..create_test_version_info()
        };
        let info2 = VersionInfo {
            latest: Some("2.0.0".to_string()),
            ..create_test_version_info()
        };

        cache.insert("test:package".to_string(), info1);
        cache.insert("test:package".to_string(), info2);

        let retrieved = cache.get("test:package").unwrap();
        assert_eq!(retrieved.latest, Some("2.0.0".to_string()));
    }
}
