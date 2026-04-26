//! SQLite-backed advisory cache (L2 layer).

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::Pool;

use crate::cache::sqlite_manager::SqliteConnectionManager;

/// Default TTL for stored advisories (24 hours).
pub const DEFAULT_ADVISORY_TTL_SECS: i64 = 86_400;

#[cfg(test)]
static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// SQLite-backed cache for OSV advisory results.
pub struct SqliteAdvisoryCache {
    pool: Arc<Pool<SqliteConnectionManager>>,
    ttl_secs: i64,
}

impl SqliteAdvisoryCache {
    /// Build a cache at the default location (`~/.cache/dependi/advisory_cache.db`).
    pub fn new() -> anyhow::Result<Self> {
        let cache_dir = Self::cache_dir()?;
        std::fs::create_dir_all(&cache_dir)?;
        let path = cache_dir.join("advisory_cache.db");
        Self::with_path(path, DEFAULT_ADVISORY_TTL_SECS)
    }

    /// Build a cache at a custom path, with a custom TTL.
    pub fn with_path(path: PathBuf, ttl_secs: i64) -> anyhow::Result<Self> {
        let manager = SqliteConnectionManager::file_with_config(&path, 5_000, 64_000);
        let pool = Pool::builder()
            .max_size(4)
            .min_idle(Some(1))
            .connection_timeout(Duration::from_secs(5))
            .idle_timeout(Some(Duration::from_secs(600)))
            .max_lifetime(Some(Duration::from_secs(1800)))
            .build(manager)?;

        let cache = Self {
            pool: Arc::new(pool),
            ttl_secs,
        };
        cache.init_schema()?;
        Ok(cache)
    }

    /// In-memory cache for tests.
    #[cfg(test)]
    pub fn in_memory() -> anyhow::Result<Self> {
        let id = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let uri = format!("file:advisorymemdb{id}?mode=memory&cache=shared");
        let manager = SqliteConnectionManager::in_memory(&uri);
        let pool = Pool::builder().max_size(4).build(manager)?;
        let cache = Self {
            pool: Arc::new(pool),
            ttl_secs: DEFAULT_ADVISORY_TTL_SECS,
        };
        cache.init_schema_memory()?;
        Ok(cache)
    }

    /// Construct a cache with a custom TTL (used by negative-cache wiring).
    pub fn with_ttl_secs(self, ttl_secs: i64) -> Self {
        Self { ttl_secs, ..self }
    }

    fn cache_dir() -> anyhow::Result<PathBuf> {
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
        Ok(cache_dir.join("dependi"))
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS advisories (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                inserted_at INTEGER NOT NULL,
                ttl_secs INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_advisory_expiry \
             ON advisories(inserted_at, ttl_secs)",
            [],
        )?;
        tracing::debug!(
            ttl_secs = self.ttl_secs,
            "SqliteAdvisoryCache schema initialised"
        );
        Ok(())
    }

    #[cfg(test)]
    fn init_schema_memory(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS advisories (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                inserted_at INTEGER NOT NULL,
                ttl_secs INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_advisory_expiry \
             ON advisories(inserted_at, ttl_secs)",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
fn current_timestamp() -> i64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    nanos.min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_constructor_creates_table_and_index() {
        let cache = SqliteAdvisoryCache::in_memory().expect("open in-memory cache");
        let conn = cache.pool.get().expect("get conn");
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'advisories' AND type = 'table'",
                [],
                |row| row.get(0),
            )
            .expect("query table");
        assert_eq!(count, 1);

        let idx: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master \
                 WHERE name = 'idx_advisory_expiry' AND type = 'index'",
                [],
                |row| row.get(0),
            )
            .expect("query index");
        assert_eq!(idx, 1);
    }

    #[tokio::test]
    async fn current_timestamp_is_positive_and_monotonic() {
        let a = current_timestamp();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let b = current_timestamp();
        assert!(a > 0);
        assert!(b >= a);
    }

    #[test]
    fn with_ttl_secs_overrides_default() {
        let cache = SqliteAdvisoryCache::in_memory()
            .expect("open in-memory cache")
            .with_ttl_secs(42);
        assert_eq!(cache.ttl_secs, 42);
    }
}
