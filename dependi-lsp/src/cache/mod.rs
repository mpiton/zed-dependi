//! Cache layer for package version information

use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

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
#[derive(Clone)]
pub struct MemoryCache {
    entries: Arc<DashMap<String, CacheEntry>>,
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
            entries: Arc::new(DashMap::new()),
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

impl MemoryCache {
    pub fn cleanup_expired(&self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, entry| !entry.is_expired());
        let removed = before - self.entries.len();
        if removed > 0 {
            tracing::debug!(
                "Cleaned up {} expired cache entries ({} remaining)",
                removed,
                self.entries.len()
            );
        }
        removed
    }

    pub fn stats(&self) -> CacheStats {
        let total = self.entries.len();
        let expired = self.entries.iter().filter(|e| e.is_expired()).count();
        CacheStats {
            total_entries: total,
            expired_entries: expired,
            valid_entries: total.saturating_sub(expired),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            ttl,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub expired_entries: usize,
    pub valid_entries: usize,
}

impl Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CacheStats {{ total: {}, expired: {}, valid: {} }}",
            self.total_entries, self.expired_entries, self.valid_entries
        )
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

/// Cleanup interval for background task (30 minutes)
const CLEANUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

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

        let memory = MemoryCache::new();
        Self::spawn_cleanup_task(memory.clone(), sqlite.clone());

        Self { memory, sqlite }
    }

    fn spawn_cleanup_task(memory: MemoryCache, sqlite: Option<Arc<SqliteCache>>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
            interval.tick().await; // Skip immediate first tick

            loop {
                interval.tick().await;

                let stats = memory.stats();
                let removed = memory.cleanup_expired();
                if removed > 0 {
                    tracing::info!(
                        "Background cleanup: removed {} expired entries from memory cache (was: {})",
                        removed,
                        stats
                    );
                }

                if let Some(ref sqlite) = sqlite
                    && let Ok(rows) = sqlite.cleanup_expired()
                    && rows > 0
                {
                    tracing::info!(
                        "Background cleanup: removed {} expired entries from SQLite cache",
                        rows
                    );
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_version_info() -> VersionInfo {
        VersionInfo {
            latest: Some("1.0.0".to_string()),
            latest_prerelease: None,
            versions: vec!["1.0.0".to_string()],
            description: None,
            homepage: None,
            repository: None,
            license: None,
            vulnerabilities: vec![],
            deprecated: false,
            yanked: false,
            yanked_versions: vec![],
            release_dates: Default::default(),
        }
    }

    #[test]
    fn test_memory_cache_cleanup_expired() {
        let cache = MemoryCache::with_ttl(Duration::from_millis(10));

        cache.insert("key1".to_string(), create_test_version_info());
        cache.insert("key2".to_string(), create_test_version_info());

        assert_eq!(cache.len(), 2);

        // Wait for entries to expire
        std::thread::sleep(Duration::from_millis(20));

        let removed = cache.cleanup_expired();
        assert_eq!(removed, 2);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_memory_cache_cleanup_partial() {
        let cache = MemoryCache::with_ttl(Duration::from_millis(50));

        cache.insert("key1".to_string(), create_test_version_info());

        // Wait for first entry to almost expire
        std::thread::sleep(Duration::from_millis(30));

        // Insert second entry
        cache.insert("key2".to_string(), create_test_version_info());

        // Wait for first to expire but not second
        std::thread::sleep(Duration::from_millis(30));

        let removed = cache.cleanup_expired();
        assert_eq!(removed, 1);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("key2").is_some());
    }

    #[test]
    fn test_memory_cache_stats() {
        let cache = MemoryCache::with_ttl(Duration::from_millis(10));

        cache.insert("key1".to_string(), create_test_version_info());
        cache.insert("key2".to_string(), create_test_version_info());

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.expired_entries, 0);
        assert_eq!(stats.valid_entries, 2);

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(20));

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.expired_entries, 2);
        assert_eq!(stats.valid_entries, 0);
    }

    #[test]
    fn test_cache_stats_display() {
        let stats = CacheStats {
            total_entries: 10,
            expired_entries: 3,
            valid_entries: 7,
        };
        let display = format!("{}", stats);
        assert!(display.contains("total: 10"));
        assert!(display.contains("expired: 3"));
        assert!(display.contains("valid: 7"));
    }

    #[test]
    fn test_memory_cache_is_empty() {
        let cache = MemoryCache::with_ttl(Duration::from_secs(60));
        assert!(cache.is_empty());
        cache.insert("key".to_string(), create_test_version_info());
        assert!(!cache.is_empty());
    }
}
