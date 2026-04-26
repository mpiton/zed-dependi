//! RustSec advisory cache (issue #237)
//!
//! Caches the result of `OSV GET /vulns/{id}` to avoid redundant network
//! requests for the same RUSTSEC advisory across LSP sessions.

pub mod memory;
pub mod sqlite;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub use memory::{AdvisoryCacheStats, DEFAULT_ADVISORY_TTL, MemoryAdvisoryCache};
pub use sqlite::{DEFAULT_ADVISORY_TTL_SECS, SqliteAdvisoryCache};

/// Cached classification of a single OSV advisory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AdvisoryKind {
    /// Advisory exists at OSV; we recorded the parts we need.
    Found {
        summary: Option<String>,
        unmaintained: bool,
    },
    /// Advisory ID returned 404 from OSV.
    NotFound,
}

/// One cache entry: an advisory ID plus its classification and fetch time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedAdvisory {
    pub id: String,
    pub kind: AdvisoryKind,
    pub fetched_at: SystemTime,
}

/// Read-only access to the advisory cache.
///
/// Mirrors [`crate::cache::ReadCache`] but specialised for advisory entries.
#[allow(async_fn_in_trait)]
pub trait AdvisoryReadCache: Send + Sync {
    /// Fetch a cached advisory. Returns `None` on miss or expiry.
    async fn get(&self, advisory_id: &str) -> Option<CachedAdvisory>;

    /// Convenience wrapper around `get` for existence checks.
    async fn contains(&self, advisory_id: &str) -> bool {
        self.get(advisory_id).await.is_some()
    }
}

/// Write access to the advisory cache.
///
/// Mirrors [`crate::cache::WriteCache`] but specialised for advisory entries.
/// Unlike `WriteCache`, the key is not passed separately — `CachedAdvisory`
/// carries its own `id`.
#[allow(async_fn_in_trait)]
pub trait AdvisoryWriteCache: AdvisoryReadCache {
    /// Insert (or replace) an advisory entry.
    ///
    /// The `advisory.id` field acts as the cache key; unlike
    /// [`crate::cache::WriteCache::insert`] the key is not passed separately.
    async fn insert(&self, advisory: CachedAdvisory);

    /// Remove a single advisory entry.
    async fn remove(&self, advisory_id: &str);

    /// Remove every entry from the cache.
    async fn clear(&self);
}

impl<T: AdvisoryReadCache> AdvisoryReadCache for Arc<T> {
    async fn get(&self, advisory_id: &str) -> Option<CachedAdvisory> {
        (**self).get(advisory_id).await
    }

    async fn contains(&self, advisory_id: &str) -> bool {
        (**self).contains(advisory_id).await
    }
}

impl<T: AdvisoryWriteCache> AdvisoryWriteCache for Arc<T> {
    async fn insert(&self, advisory: CachedAdvisory) {
        (**self).insert(advisory).await
    }

    async fn remove(&self, advisory_id: &str) {
        (**self).remove(advisory_id).await
    }

    async fn clear(&self) {
        (**self).clear().await
    }
}

/// No-op cache used when caching is disabled via configuration.
///
/// All reads return `None`, all writes are silently dropped.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullAdvisoryCache;

impl AdvisoryReadCache for NullAdvisoryCache {
    async fn get(&self, _advisory_id: &str) -> Option<CachedAdvisory> {
        None
    }
}

impl AdvisoryWriteCache for NullAdvisoryCache {
    async fn insert(&self, _advisory: CachedAdvisory) {}
    async fn remove(&self, _advisory_id: &str) {}
    async fn clear(&self) {}
}

/// Cleanup interval for the hybrid cache background task (30 minutes).
pub const ADVISORY_CLEANUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Two-tier advisory cache: in-memory L1 backed by SQLite L2.
pub struct HybridAdvisoryCache {
    memory: memory::MemoryAdvisoryCache,
    sqlite: Option<Arc<sqlite::SqliteAdvisoryCache>>,
}

impl HybridAdvisoryCache {
    /// Construct a hybrid cache, attempting to open the default SQLite path.
    pub fn new() -> Self {
        let sqlite = match sqlite::SqliteAdvisoryCache::new() {
            Ok(cache) => {
                tracing::info!("Advisory SQLite cache initialised");
                Some(Arc::new(cache))
            }
            Err(err) => {
                tracing::warn!(
                    "Advisory SQLite cache unavailable, using memory only: {}",
                    err
                );
                None
            }
        };
        let memory = memory::MemoryAdvisoryCache::new();
        Self { memory, sqlite }
    }

    /// Build directly from prepared layers (used by tests and by callers
    /// that have constructed a custom SQLite cache).
    pub fn from_parts(
        memory: memory::MemoryAdvisoryCache,
        sqlite: Option<Arc<sqlite::SqliteAdvisoryCache>>,
    ) -> Self {
        Self { memory, sqlite }
    }
}

impl Default for HybridAdvisoryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl AdvisoryReadCache for HybridAdvisoryCache {
    async fn get(&self, advisory_id: &str) -> Option<CachedAdvisory> {
        if let Some(value) = self.memory.get(advisory_id).await {
            return Some(value);
        }

        if let Some(ref sqlite) = self.sqlite
            && let Some(value) = sqlite.get(advisory_id).await
        {
            self.memory.insert(value.clone()).await;
            return Some(value);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemoryAdvisoryCache;
    use super::sqlite::SqliteAdvisoryCache;
    use super::*;

    fn sample_found(id: &str) -> CachedAdvisory {
        CachedAdvisory {
            id: id.to_string(),
            kind: AdvisoryKind::Found {
                summary: Some("unmaintained".to_string()),
                unmaintained: true,
            },
            fetched_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn hybrid_l1_hit_returns_without_consulting_l2() {
        let memory = MemoryAdvisoryCache::new();
        let sqlite = Arc::new(SqliteAdvisoryCache::in_memory().expect("in-memory sqlite"));
        let advisory = sample_found("RUSTSEC-2020-0036");
        memory.insert(advisory.clone()).await;
        // Note: deliberately NOT inserting into SQLite.
        let hybrid = HybridAdvisoryCache::from_parts(memory, Some(sqlite));
        assert_eq!(hybrid.get("RUSTSEC-2020-0036").await, Some(advisory));
    }

    #[test]
    fn cached_advisory_round_trips_through_json() {
        let advisory = CachedAdvisory {
            id: "RUSTSEC-2020-0036".to_string(),
            kind: AdvisoryKind::Found {
                summary: Some("failure crate is unmaintained".to_string()),
                unmaintained: true,
            },
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&advisory).expect("serialize");
        let back: CachedAdvisory = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(advisory, back);
    }

    #[test]
    fn not_found_kind_round_trips() {
        let advisory = CachedAdvisory {
            id: "RUSTSEC-9999-0001".to_string(),
            kind: AdvisoryKind::NotFound,
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&advisory).expect("serialize");
        let back: CachedAdvisory = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(advisory, back);
    }

    #[tokio::test]
    async fn arc_blanket_impl_forwards_reads() {
        struct DummyCache {
            value: Option<CachedAdvisory>,
        }

        impl AdvisoryReadCache for DummyCache {
            async fn get(&self, _id: &str) -> Option<CachedAdvisory> {
                self.value.clone()
            }
        }

        let advisory = CachedAdvisory {
            id: "RUSTSEC-2020-0036".to_string(),
            kind: AdvisoryKind::NotFound,
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        let cache: Arc<DummyCache> = Arc::new(DummyCache {
            value: Some(advisory.clone()),
        });
        assert_eq!(cache.get("anything").await, Some(advisory.clone()));
        assert!(cache.contains("anything").await);
    }

    #[tokio::test]
    async fn null_cache_get_returns_none() {
        let cache = NullAdvisoryCache;
        assert_eq!(cache.get("RUSTSEC-2020-0036").await, None);
        assert!(!cache.contains("RUSTSEC-2020-0036").await);
    }

    #[tokio::test]
    async fn null_cache_writes_are_noop() {
        let cache = NullAdvisoryCache;
        cache
            .insert(CachedAdvisory {
                id: "RUSTSEC-2020-0036".to_string(),
                kind: AdvisoryKind::NotFound,
                fetched_at: SystemTime::UNIX_EPOCH,
            })
            .await;
        cache.remove("RUSTSEC-2020-0036").await;
        cache.clear().await;
        assert_eq!(cache.get("RUSTSEC-2020-0036").await, None);
    }
}
