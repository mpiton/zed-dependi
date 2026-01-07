//! SQLite persistent cache for package version information with connection pooling

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

use crate::cache::{ReadCache, WriteCache};
use crate::registries::VersionInfo;

/// Default TTL for cache entries (1 hour)
const DEFAULT_TTL_SECS: i64 = 3600;

#[cfg(test)]
static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Configuration for SQLite cache pool
#[derive(Debug, Clone)]
pub struct SqliteCacheConfig {
    /// Maximum number of connections in the pool
    pub max_pool_size: u32,
    /// Minimum number of idle connections to maintain
    pub min_idle_connections: u32,
    /// Timeout in seconds for acquiring a connection
    pub connection_timeout_secs: u64,
    /// SQLite busy timeout in milliseconds
    pub busy_timeout_ms: u32,
    /// SQLite cache size in kilobytes
    pub cache_size_kb: i64,
    /// Time-to-live for cache entries in seconds
    pub ttl_secs: i64,
}

impl Default for SqliteCacheConfig {
    fn default() -> Self {
        Self {
            max_pool_size: 10,
            min_idle_connections: 2,
            connection_timeout_secs: 5,
            busy_timeout_ms: 5000,
            cache_size_kb: 64000,
            ttl_secs: DEFAULT_TTL_SECS,
        }
    }
}

/// SQLite-based persistent cache with connection pooling
pub struct SqliteCache {
    pool: Arc<Pool<SqliteConnectionManager>>,
    ttl_secs: i64,
}

impl SqliteCache {
    /// Create a new SQLite cache at the default location (~/.cache/dependi/cache.db)
    pub fn new() -> anyhow::Result<Self> {
        Self::with_config(SqliteCacheConfig::default())
    }

    /// Create a new SQLite cache with custom configuration
    pub fn with_config(config: SqliteCacheConfig) -> anyhow::Result<Self> {
        let cache_dir = Self::cache_dir()?;
        std::fs::create_dir_all(&cache_dir)?;
        let db_path = cache_dir.join("cache.db");
        Self::with_path_and_config(db_path, config)
    }

    /// Create a new SQLite cache at a custom path with custom configuration
    pub fn with_path_and_config(path: PathBuf, config: SqliteCacheConfig) -> anyhow::Result<Self> {
        let busy_timeout_ms = config.busy_timeout_ms;
        let cache_size_kb = config.cache_size_kb;

        let manager = SqliteConnectionManager::file(&path).with_init(move |conn| {
            let pragmas = format!(
                "PRAGMA busy_timeout={};
                 PRAGMA synchronous=NORMAL;
                 PRAGMA cache_size=-{};",
                busy_timeout_ms, cache_size_kb
            );
            conn.execute_batch(&pragmas)?;
            Ok(())
        });

        let pool = Pool::builder()
            .max_size(config.max_pool_size)
            .min_idle(Some(config.min_idle_connections))
            .connection_timeout(Duration::from_secs(config.connection_timeout_secs))
            .idle_timeout(Some(Duration::from_secs(600)))
            .max_lifetime(Some(Duration::from_secs(1800)))
            .build(manager)?;

        let cache = Self {
            pool: Arc::new(pool),
            ttl_secs: config.ttl_secs,
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
        let config = SqliteCacheConfig::default();

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
            ttl_secs: config.ttl_secs,
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

    /// Get a connection from the pool, returning None if unavailable
    fn get_conn(&self) -> Option<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().ok()
    }

    /// Initialize the database schema with WAL mode
    ///
    /// Note: Per-connection PRAGMAs (busy_timeout, synchronous, cache_size)
    /// are applied via with_init() on every new connection from the pool.
    /// Only WAL mode (database-level) is set here.
    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

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
}

impl ReadCache for SqliteCache {
    fn get(&self, key: &str) -> Option<VersionInfo> {
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
}

impl WriteCache for SqliteCache {
    fn insert(&self, key: String, value: VersionInfo) {
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

    fn remove(&self, key: &str) {
        let Some(conn) = self.get_conn() else {
            return;
        };
        let _ = conn.execute("DELETE FROM packages WHERE key = ?", [key]);
    }

    fn clear(&self) {
        let Some(conn) = self.get_conn() else {
            return;
        };
        let _ = conn.execute("DELETE FROM packages", []);
    }
}

impl SqliteCache {
    /// Insert multiple values in a single transaction
    #[cfg(test)]
    pub fn insert_batch(&self, entries: Vec<(String, VersionInfo)>) -> anyhow::Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let now = current_timestamp();
        let mut count = 0;

        for (key, value) in entries {
            let data = serde_json::to_string(&value)?;
            tx.execute(
                "INSERT OR REPLACE INTO packages (key, data, inserted_at, ttl_secs) VALUES (?, ?, ?, ?)",
                params![key, data, now, self.ttl_secs],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    /// Remove a value from the cache, returning whether it existed
    #[cfg(test)]
    pub fn remove_with_result(&self, key: &str) -> bool {
        let Some(conn) = self.get_conn() else {
            return false;
        };
        conn.execute("DELETE FROM packages WHERE key = ?", [key])
            .map(|rows| rows > 0)
            .unwrap_or(false)
    }

    /// Clear all entries from the cache, returning the count
    #[cfg(test)]
    pub fn clear_with_count(&self) -> anyhow::Result<usize> {
        let conn = self.pool.get()?;
        let rows = conn.execute("DELETE FROM packages", [])?;
        Ok(rows)
    }

    /// Remove expired entries from the cache
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
    /// Total number of connections in the pool
    pub connections: u32,
    /// Number of idle connections available
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

    #[test]
    fn test_remove() {
        let cache = SqliteCache::in_memory().unwrap();
        let info = create_test_version_info();

        cache.insert("test:package".to_string(), info);
        assert!(cache.get("test:package").is_some());

        let removed = cache.remove_with_result("test:package");
        assert!(removed);
        assert!(cache.get("test:package").is_none());

        let removed_again = cache.remove_with_result("test:package");
        assert!(!removed_again);
    }

    #[test]
    fn test_clear() {
        let cache = SqliteCache::in_memory().unwrap();
        let info = create_test_version_info();

        cache.insert("pkg1".to_string(), info.clone());
        cache.insert("pkg2".to_string(), info.clone());
        cache.insert("pkg3".to_string(), info);

        let cleared = cache.clear_with_count().unwrap();
        assert_eq!(cleared, 3);

        assert!(cache.get("pkg1").is_none());
        assert!(cache.get("pkg2").is_none());
        assert!(cache.get("pkg3").is_none());
    }

    #[test]
    fn test_insert_batch() {
        let cache = SqliteCache::in_memory().unwrap();

        let entries: Vec<(String, VersionInfo)> = (0..10)
            .map(|i| {
                let mut info = create_test_version_info();
                info.latest = Some(format!("{}.0.0", i));
                (format!("pkg{}", i), info)
            })
            .collect();

        let count = cache.insert_batch(entries).unwrap();
        assert_eq!(count, 10);

        for i in 0..10 {
            let retrieved = cache.get(&format!("pkg{}", i)).unwrap();
            assert_eq!(retrieved.latest, Some(format!("{}.0.0", i)));
        }
    }

    #[test]
    fn test_insert_batch_empty() {
        let cache = SqliteCache::in_memory().unwrap();
        let count = cache.insert_batch(vec![]).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(SqliteCache::in_memory().unwrap());

        for i in 0..20 {
            let mut info = create_test_version_info();
            info.latest = Some(format!("{}.0.0", i));
            cache.insert(format!("pkg{}", i), info);
        }

        let handles: Vec<_> = (0..10)
            .map(|thread_id| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for i in 0..20 {
                        let key = format!("pkg{}", i);
                        let result = cache.get(&key);
                        assert!(
                            result.is_some(),
                            "Thread {} failed to read {}",
                            thread_id,
                            key
                        );
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }

    #[test]
    fn test_concurrent_writes() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(SqliteCache::in_memory().unwrap());

        let handles: Vec<_> = (0..5)
            .map(|thread_id| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for i in 0..10 {
                        let key = format!("thread{}:pkg{}", thread_id, i);
                        let mut info = create_test_version_info();
                        info.latest = Some(format!("{}.{}.0", thread_id, i));
                        cache.insert(key, info);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        for thread_id in 0..5 {
            for i in 0..10 {
                let key = format!("thread{}:pkg{}", thread_id, i);
                let result = cache.get(&key);
                assert!(result.is_some(), "Missing key: {}", key);
            }
        }
    }

    #[test]
    fn test_concurrent_mixed_operations() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(SqliteCache::in_memory().unwrap());

        for i in 0..50 {
            let mut info = create_test_version_info();
            info.latest = Some(format!("{}.0.0", i));
            cache.insert(format!("pkg{}", i), info);
        }

        let handles: Vec<_> = (0..10)
            .map(|thread_id| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    for i in 0..50 {
                        match thread_id % 3 {
                            0 => {
                                let _ = cache.get(&format!("pkg{}", i));
                            }
                            1 => {
                                let mut info = create_test_version_info();
                                info.latest = Some(format!("updated-{}-{}", thread_id, i));
                                cache.insert(format!("pkg{}", i), info);
                            }
                            _ => {
                                let mut info = create_test_version_info();
                                info.latest = Some(format!("new-{}-{}", thread_id, i));
                                cache.insert(format!("new-pkg-{}-{}", thread_id, i), info);
                            }
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }

    #[test]
    fn test_config_default() {
        let config = SqliteCacheConfig::default();
        assert_eq!(config.max_pool_size, 10);
        assert_eq!(config.min_idle_connections, 2);
        assert_eq!(config.busy_timeout_ms, 5000);
        assert_eq!(config.cache_size_kb, 64000);
        assert_eq!(config.ttl_secs, DEFAULT_TTL_SECS);
    }
}
