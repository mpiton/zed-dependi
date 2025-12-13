//! Vulnerability cache for tracking queried packages
//!
//! Tracks which packages have been queried for vulnerabilities to avoid
//! redundant API calls. The actual vulnerability data is stored in the
//! version cache alongside version information.

use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::Ecosystem;

/// Default TTL for vulnerability cache (6 hours)
const DEFAULT_VULN_CACHE_TTL: Duration = Duration::from_secs(6 * 3600);

/// Cache key for vulnerability lookups
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct VulnCacheKey {
    /// Target ecosystem
    pub ecosystem: Ecosystem,
    /// Package name
    pub package_name: String,
    /// Package version
    pub version: String,
}

impl VulnCacheKey {
    /// Create a new cache key
    pub fn new(ecosystem: Ecosystem, package_name: &str, version: &str) -> Self {
        Self {
            ecosystem,
            package_name: package_name.to_string(),
            version: version.to_string(),
        }
    }
}

/// Tracks when a package was queried for vulnerabilities
struct VulnCacheEntry {
    /// When the entry was inserted
    inserted_at: Instant,
}

/// Tracks which packages have been queried for vulnerabilities
///
/// This is a "seen set" with TTL - it prevents redundant API calls to OSV.dev
/// by tracking which package@version combinations have already been queried.
/// The actual vulnerability data is stored in the version cache.
pub struct VulnerabilityCache {
    /// Cache entries (package key -> query timestamp)
    entries: DashMap<VulnCacheKey, VulnCacheEntry>,
    /// Cache TTL
    ttl: Duration,
}

impl VulnerabilityCache {
    /// Create a new cache with default TTL (6 hours)
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            ttl: DEFAULT_VULN_CACHE_TTL,
        }
    }

    /// Create a cache with custom TTL in seconds
    #[cfg(test)]
    pub fn with_ttl(ttl_secs: u64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Mark a package as having been queried for vulnerabilities
    pub fn insert(&self, key: VulnCacheKey) {
        self.entries.insert(
            key,
            VulnCacheEntry {
                inserted_at: Instant::now(),
            },
        );
    }

    /// Check if a package has been queried (and the query hasn't expired)
    pub fn contains(&self, key: &VulnCacheKey) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| entry.inserted_at.elapsed() < self.ttl)
    }

    /// Remove expired entries from cache
    #[cfg(test)]
    pub fn cleanup(&self) {
        self.entries
            .retain(|_, entry| entry.inserted_at.elapsed() < self.ttl);
    }

    /// Clear all entries from cache
    #[cfg(test)]
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Get the number of entries in the cache
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for VulnerabilityCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_contains() {
        let cache = VulnerabilityCache::new();
        let key = VulnCacheKey::new(Ecosystem::Npm, "lodash", "4.17.0");

        assert!(!cache.contains(&key));
        cache.insert(key.clone());
        assert!(cache.contains(&key));
    }

    #[test]
    fn test_cache_clear() {
        let cache = VulnerabilityCache::new();
        let key = VulnCacheKey::new(Ecosystem::PyPI, "requests", "2.28.0");

        cache.insert(key.clone());
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_with_custom_ttl() {
        let cache = VulnerabilityCache::with_ttl(3600); // 1 hour
        assert_eq!(cache.ttl, Duration::from_secs(3600));
    }
}
