//! Vulnerability cache with configurable TTL
//!
//! Provides in-memory caching for vulnerability data with a default
//! 6-hour TTL to reduce API calls.

use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::Ecosystem;
use crate::registries::Vulnerability;

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

    /// Convert to a string key for SQLite storage
    pub fn to_sqlite_key(&self) -> String {
        format!(
            "vuln:{}:{}:{}",
            self.ecosystem.as_osv_str(),
            self.package_name,
            self.version
        )
    }
}

/// Entry in the vulnerability cache
struct VulnCacheEntry {
    /// Cached vulnerabilities
    vulnerabilities: Vec<Vulnerability>,
    /// When the entry was inserted
    inserted_at: Instant,
}

/// In-memory vulnerability cache with TTL
pub struct VulnerabilityCache {
    /// Cache entries
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

    /// Create a cache with custom TTL
    pub fn with_ttl(ttl_secs: u64) -> Self {
        Self {
            entries: DashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get vulnerabilities from cache if present and not expired
    pub fn get(&self, key: &VulnCacheKey) -> Option<Vec<Vulnerability>> {
        self.entries.get(key).and_then(|entry| {
            if entry.inserted_at.elapsed() < self.ttl {
                Some(entry.vulnerabilities.clone())
            } else {
                None
            }
        })
    }

    /// Insert vulnerabilities into cache
    pub fn insert(&self, key: VulnCacheKey, vulnerabilities: Vec<Vulnerability>) {
        self.entries.insert(
            key,
            VulnCacheEntry {
                vulnerabilities,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Check if a key is in the cache and not expired
    pub fn contains(&self, key: &VulnCacheKey) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.inserted_at.elapsed() < self.ttl)
    }

    /// Remove expired entries from cache
    pub fn cleanup(&self) {
        self.entries
            .retain(|_, entry| entry.inserted_at.elapsed() < self.ttl);
    }

    /// Clear all entries from cache
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Get the number of entries in the cache
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
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
    use crate::registries::VulnerabilitySeverity;

    #[test]
    fn test_cache_insert_and_get() {
        let cache = VulnerabilityCache::new();
        let key = VulnCacheKey::new(Ecosystem::Npm, "lodash", "4.17.0");

        let vulns = vec![Vulnerability {
            id: "CVE-2021-23337".to_string(),
            severity: VulnerabilitySeverity::High,
            description: "Prototype pollution".to_string(),
            url: Some("https://nvd.nist.gov/vuln/detail/CVE-2021-23337".to_string()),
        }];

        cache.insert(key.clone(), vulns.clone());

        let retrieved = cache.get(&key).unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].id, "CVE-2021-23337");
    }

    #[test]
    fn test_cache_contains() {
        let cache = VulnerabilityCache::new();
        let key = VulnCacheKey::new(Ecosystem::CratesIo, "serde", "1.0.0");

        assert!(!cache.contains(&key));

        cache.insert(key.clone(), vec![]);

        assert!(cache.contains(&key));
    }

    #[test]
    fn test_cache_clear() {
        let cache = VulnerabilityCache::new();
        let key = VulnCacheKey::new(Ecosystem::PyPI, "requests", "2.28.0");

        cache.insert(key.clone(), vec![]);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_key_sqlite() {
        let key = VulnCacheKey::new(Ecosystem::Npm, "lodash", "4.17.21");
        assert_eq!(key.to_sqlite_key(), "vuln:npm:lodash:4.17.21");
    }

    #[test]
    fn test_cache_with_custom_ttl() {
        let cache = VulnerabilityCache::with_ttl(3600); // 1 hour
        assert_eq!(cache.ttl, Duration::from_secs(3600));
    }
}
