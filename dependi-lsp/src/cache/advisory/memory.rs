//! In-memory advisory cache (L1 layer) backed by [`DashMap`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::{AdvisoryReadCache, AdvisoryWriteCache, CachedAdvisory};

/// Default TTL for cached advisory entries (24 hours).
pub const DEFAULT_ADVISORY_TTL: Duration = Duration::from_secs(86_400);

#[derive(Clone, Debug)]
struct MemoryAdvisoryEntry {
    advisory: CachedAdvisory,
    inserted_at: Instant,
    ttl: Duration,
}

impl MemoryAdvisoryEntry {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

/// Thread-safe, in-memory advisory cache built on a [`DashMap`].
#[derive(Clone)]
pub struct MemoryAdvisoryCache {
    entries: Arc<DashMap<String, MemoryAdvisoryEntry>>,
    ttl: Duration,
}

impl Default for MemoryAdvisoryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAdvisoryCache {
    /// Build a cache with the default 24-hour TTL.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            ttl: DEFAULT_ADVISORY_TTL,
        }
    }

    /// Build a cache with a custom TTL (used by tests and for negative caching).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            ttl,
        }
    }
}

impl AdvisoryReadCache for MemoryAdvisoryCache {
    async fn get(&self, advisory_id: &str) -> Option<CachedAdvisory> {
        self.entries
            .get(advisory_id)
            .and_then(|entry| (!entry.is_expired()).then(|| entry.advisory.clone()))
    }
}

impl AdvisoryWriteCache for MemoryAdvisoryCache {
    async fn insert(&self, advisory: CachedAdvisory) {
        self.entries.insert(
            advisory.id.clone(),
            MemoryAdvisoryEntry {
                advisory,
                inserted_at: Instant::now(),
                ttl: self.ttl,
            },
        );
    }

    async fn remove(&self, advisory_id: &str) {
        self.entries.remove(advisory_id);
    }

    async fn clear(&self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::super::AdvisoryKind;
    use super::*;

    fn sample_found() -> CachedAdvisory {
        CachedAdvisory {
            id: "RUSTSEC-2020-0036".to_string(),
            kind: AdvisoryKind::Found {
                summary: Some("unmaintained".to_string()),
                unmaintained: true,
            },
            fetched_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn insert_and_get_round_trip() {
        let cache = MemoryAdvisoryCache::new();
        let advisory = sample_found();
        cache.insert(advisory.clone()).await;
        assert_eq!(cache.get(&advisory.id).await, Some(advisory));
    }

    #[tokio::test]
    async fn missing_id_returns_none() {
        let cache = MemoryAdvisoryCache::new();
        assert!(cache.get("RUSTSEC-9999-9999").await.is_none());
    }

    fn sample_not_found(id: &str) -> CachedAdvisory {
        CachedAdvisory {
            id: id.to_string(),
            kind: AdvisoryKind::NotFound,
            fetched_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn second_insert_overwrites_first() {
        let cache = MemoryAdvisoryCache::new();
        cache.insert(sample_found()).await;
        let replacement = CachedAdvisory {
            id: "RUSTSEC-2020-0036".to_string(),
            kind: AdvisoryKind::Found {
                summary: None,
                unmaintained: false,
            },
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        cache.insert(replacement.clone()).await;
        assert_eq!(cache.get(&replacement.id).await, Some(replacement));
    }

    #[tokio::test]
    async fn remove_deletes_entry() {
        let cache = MemoryAdvisoryCache::new();
        cache.insert(sample_found()).await;
        cache.remove("RUSTSEC-2020-0036").await;
        assert!(cache.get("RUSTSEC-2020-0036").await.is_none());
    }

    #[tokio::test]
    async fn clear_empties_cache() {
        let cache = MemoryAdvisoryCache::new();
        cache.insert(sample_found()).await;
        cache.insert(sample_not_found("RUSTSEC-9999-0001")).await;
        cache.clear().await;
        assert!(cache.get("RUSTSEC-2020-0036").await.is_none());
        assert!(cache.get("RUSTSEC-9999-0001").await.is_none());
    }
}
