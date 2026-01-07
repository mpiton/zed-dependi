//! SQLite persistent cache for package version information with connection pooling

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::registries::VersionInfo;

/// Default TTL for cache entries (1 hour)
const DEFAULT_TTL_SECS: i64 = 3600;

#[cfg(test)]
static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// SQLite-based persistent cache with connection pooling
pub struct SqliteCache {
    pool: Arc<Pool<SqliteConnectionManager>>,
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
        let manager = SqliteConnectionManager::file(&path);

        let pool = Pool::builder()
            .max_size(10)
            .min_idle(Some(2))
            .connection_timeout(Duration::from_secs(5))
            .idle_timeout(Some(Duration::from_secs(600)))
            .max_lifetime(Some(Duration::from_secs(1800)))
            .build(manager)?;

        let cache = Self {
            pool: Arc::new(pool),
            ttl_secs: DEFAULT_TTL_SECS,
        };

        cache.init_schema()?;
        cache.cleanup_expired()?;

        let state = cache.pool_state();
        tracing::debug!(
            connections = state.connections,
            idle = state.idle_connections,
            "SQLite cache pool initialized"
        );

        Ok(cache)
    }

    /// Create an in-memory cache (for testing)
    ///
    /// Uses a shared in-memory database URI so all pooled connections access
    /// the same database. Each call generates a unique database name to avoid
    /// conflicts between tests.
    #[cfg(test)]
    pub fn in_memory() -> anyhow::Result<Self> {
        let db_id = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let uri = format!("file:memdb{}?mode=memory&cache=shared", db_id);

        let manager = SqliteConnectionManager::file(&uri).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA busy_timeout=5000;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA cache_size=-64000;",
            )?;
            Ok(())
        });

        let pool = Pool::builder().max_size(5).build(manager)?;

        let cache = Self {
            pool: Arc::new(pool),
            ttl_secs: DEFAULT_TTL_SECS,
        };

        cache.init_schema_memory()?;
        Ok(cache)
    }

    /// Initialize schema for in-memory database (no WAL mode)
    #[cfg(test)]
    fn init_schema_memory(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;

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

    /// Get the cache directory
    fn cache_dir() -> anyhow::Result<PathBuf> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        Ok(cache_dir.join("dependi"))
    }

    fn get_conn(&self) -> Option<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().ok()
    }

    /// Initialize the database schema with WAL mode and optimized PRAGMAs
    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-64000;",
        )?;

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
        let conn = self.get_conn()?;
        let now = current_timestamp();

        let result: Result<(String, i64, i64), _> = conn.query_row(
            "SELECT data, inserted_at, ttl_secs FROM packages WHERE key = ?",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        match result {
            Ok((data, inserted_at, ttl_secs)) => {
                if now > inserted_at + ttl_secs {
                    let _ = conn.execute("DELETE FROM packages WHERE key = ?", [key]);
                    None
                } else {
                    serde_json::from_str(&data).ok()
                }
            }
            Err(_) => None,
        }
    }

    /// Insert a value into the cache
    pub fn insert(&self, key: String, value: VersionInfo) {
        let Some(conn) = self.get_conn() else {
            return;
        };
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
        let conn = self.pool.get()?;
        let now = current_timestamp();
        let rows = conn.execute(
            "DELETE FROM packages WHERE inserted_at + ttl_secs < ?",
            [now],
        )?;
        Ok(rows)
    }

    /// Get pool statistics for monitoring
    pub fn pool_state(&self) -> PoolState {
        let state = self.pool.state();
        PoolState {
            connections: state.connections,
            idle_connections: state.idle_connections,
        }
    }
}

/// Pool statistics for monitoring
#[derive(Debug, Clone, Copy)]
pub struct PoolState {
    pub connections: u32,
    pub idle_connections: u32,
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

    #[test]
    fn test_pool_state() {
        let cache = SqliteCache::in_memory().unwrap();
        let state = cache.pool_state();
        assert!(state.connections > 0);
    }
}
