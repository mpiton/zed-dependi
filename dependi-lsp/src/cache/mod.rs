//! Cache layer for package version information

use dashmap::DashMap;
use std::time::{Duration, Instant};

use crate::registries::VersionInfo;

/// Default TTL for cache entries (1 hour)
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Cache entry with expiration
#[derive(Debug, Clone)]
struct CacheEntry {
    data: VersionInfo,
    inserted_at: Instant,
    ttl: Duration,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

/// In-memory cache using DashMap for thread-safety
#[derive(Debug)]
pub struct MemoryCache {
    entries: DashMap<String, CacheEntry>,
    ttl: Duration,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryCache {
    /// Create a new cache with default TTL
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            ttl: DEFAULT_TTL,
        }
    }

    /// Create a new cache with custom TTL
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
        }
    }

    /// Get a value from the cache
    pub fn get(&self, key: &str) -> Option<VersionInfo> {
        self.entries.get(key).and_then(|entry| {
            if entry.is_expired() {
                None
            } else {
                Some(entry.data.clone())
            }
        })
    }

    /// Insert a value into the cache
    pub fn insert(&self, key: String, value: VersionInfo) {
        self.entries.insert(
            key,
            CacheEntry {
                data: value,
                inserted_at: Instant::now(),
                ttl: self.ttl,
            },
        );
    }

    /// Remove a value from the cache
    pub fn remove(&self, key: &str) -> Option<VersionInfo> {
        self.entries.remove(key).map(|(_, entry)| entry.data)
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Remove expired entries
    pub fn cleanup(&self) {
        self.entries.retain(|_, entry| !entry.is_expired());
    }
}

// TODO: Implement SQLite persistent cache
// pub mod sqlite;
