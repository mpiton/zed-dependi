//! Cache layer for package version information

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::registries::VersionInfo;

pub mod sqlite;

pub use sqlite::SqliteCache;

/// Trait for cache implementations
pub trait Cache: Send + Sync {
    /// Get a value from the cache
    fn get(&self, key: &str) -> Option<VersionInfo>;
}

// Implement Cache for Arc<T> where T: Cache
impl<T: Cache> Cache for Arc<T> {
    fn get(&self, key: &str) -> Option<VersionInfo> {
        (**self).get(key)
    }
}

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
}

impl Cache for MemoryCache {
    fn get(&self, key: &str) -> Option<VersionInfo> {
        self.get(key)
    }
}

/// Hybrid cache that uses memory for fast access and SQLite for persistence
pub struct HybridCache {
    memory: MemoryCache,
    sqlite: Option<Arc<SqliteCache>>,
}

impl Default for HybridCache {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridCache {
    /// Create a new hybrid cache
    pub fn new() -> Self {
        let sqlite = match SqliteCache::new() {
            Ok(cache) => {
                tracing::info!("SQLite cache initialized");
                Some(Arc::new(cache))
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to initialize SQLite cache, using memory only: {}",
                    e
                );
                None
            }
        };

        Self {
            memory: MemoryCache::new(),
            sqlite,
        }
    }

    /// Get a value from the cache (memory first, then SQLite)
    pub fn get(&self, key: &str) -> Option<VersionInfo> {
        // Fast path: check memory cache first
        if let Some(value) = self.memory.get(key) {
            return Some(value);
        }

        // Slow path: check SQLite cache
        if let Some(ref sqlite) = self.sqlite
            && let Some(value) = sqlite.get(key)
        {
            // Populate memory cache for future fast access
            self.memory.insert(key.to_string(), value.clone());
            return Some(value);
        }

        None
    }

    /// Insert a value into both caches
    pub fn insert(&self, key: String, value: VersionInfo) {
        // Insert into memory cache
        self.memory.insert(key.clone(), value.clone());

        // Insert into SQLite cache
        if let Some(ref sqlite) = self.sqlite {
            sqlite.insert(key, value);
        }
    }
}

impl Cache for HybridCache {
    fn get(&self, key: &str) -> Option<VersionInfo> {
        self.get(key)
    }
}
