//! Client for Packagist (PHP package registry)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::{Registry, VersionInfo};

/// Client for the Packagist registry
pub struct PackagistRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl PackagistRegistry {
    /// Creates a PackagistRegistry configured with the given shared HTTP client and the default Packagist API base URL.
    ///
    /// The registry's base URL is set to "https://repo.packagist.org".
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use dependi_lsp::registries::packagist::PackagistRegistry;
    ///
    /// let client = Arc::new(reqwest::Client::new());
    /// let registry = PackagistRegistry::with_client(client);
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://repo.packagist.org".to_string(),
        }
    }
}

impl Default for PackagistRegistry {
    /// Create a PackagistRegistry configured with a shared HTTP client and the default Packagist API base URL.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use dependi_lsp::registries::packagist::PackagistRegistry;
    ///
    /// let _registry = PackagistRegistry::default();
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
    }
}

// Packagist API response structures
#[derive(Debug, Deserialize)]
struct PackagistResponse {
    packages: HashMap<String, Vec<VersionEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
struct VersionEntry {
    version: String,
    description: Option<String>,
    homepage: Option<String>,
    license: Option<Vec<String>>,
    source: Option<SourceInfo>,
    /// Can be bool or string (replacement package name). We only care if it's truthy.
    abandoned: Option<serde_json::Value>,
    /// Release time
    time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SourceInfo {
    url: Option<String>,
}

impl Registry for PackagistRegistry {
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Package name format: vendor/package
        if !package_name.contains('/') {
            anyhow::bail!(
                "Invalid package name: {} (expected vendor/package)",
                package_name
            );
        }

        let url = format!("{}/p2/{}.json", self.base_url, package_name);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch package info for {}: {}",
                package_name,
                response.status()
            );
        }

        let packagist_response: PackagistResponse = response.json().await?;

        // Get versions for this package
        let entries = packagist_response
            .packages
            .get(package_name)
            .cloned()
            .unwrap_or_default();

        // Filter out dev versions and sort
        let mut versions: Vec<String> = entries
            .iter()
            .filter(|e| !is_dev_version(&e.version))
            .map(|e| e.version.clone())
            .collect();

        // Sort versions in descending order
        versions.sort_by(|a, b| compare_packagist_versions(b, a));

        // Find latest stable version
        let latest_stable = versions.iter().find(|v| !is_prerelease(v)).cloned();

        // Find latest prerelease
        let latest_prerelease = versions.iter().find(|v| is_prerelease(v)).cloned();

        // Get metadata from first (latest) entry
        let latest_entry = entries.first();

        let description = latest_entry.and_then(|e| e.description.clone());
        let homepage = latest_entry.and_then(|e| e.homepage.clone());
        let license = latest_entry
            .and_then(|e| e.license.as_ref())
            .and_then(|l| l.first())
            .cloned();
        let repository = latest_entry
            .and_then(|e| e.source.as_ref())
            .and_then(|s| s.url.clone())
            .map(|url| normalize_repo_url(&url));

        // Check if deprecated/abandoned (truthy value = abandoned)
        let deprecated = latest_entry
            .and_then(|e| e.abandoned.as_ref())
            .is_some_and(|v| !matches!(v, serde_json::Value::Bool(false)));

        // Collect release dates
        let release_dates: HashMap<String, DateTime<Utc>> = entries
            .iter()
            .filter_map(|e| {
                e.time.as_ref().and_then(|time_str| {
                    DateTime::parse_from_rfc3339(time_str)
                        .ok()
                        .map(|dt| (e.version.clone(), dt.with_timezone(&Utc)))
                })
            })
            .collect();

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description,
            homepage,
            repository,
            license,
            vulnerabilities: vec![], // TODO: Check PHP Security Advisories
            deprecated,
            yanked: false,
            yanked_versions: vec![], // Not applicable to Packagist
            release_dates,
        })
    }
}

/// Check if a version is a dev version (e.g., dev-master, dev-main)
fn is_dev_version(version: &str) -> bool {
    version.starts_with("dev-") || version.ends_with("-dev")
}

/// Check if a version is a prerelease
fn is_prerelease(version: &str) -> bool {
    let v = version.to_lowercase();
    v.contains("alpha")
        || v.contains("beta")
        || v.contains("rc")
        || v.contains("-rc")
        || v.contains("dev")
}

/// Compare Packagist versions for sorting
fn compare_packagist_versions(a: &str, b: &str) -> std::cmp::Ordering {
    // Strip 'v' prefix if present
    let a_stripped = a.strip_prefix('v').unwrap_or(a);
    let b_stripped = b.strip_prefix('v').unwrap_or(b);

    // Try parsing as semver
    match (
        semver::Version::parse(a_stripped),
        semver::Version::parse(b_stripped),
    ) {
        (Ok(va), Ok(vb)) => va.cmp(&vb),
        _ => {
            // Fallback to string comparison
            compare_version_strings(a_stripped, b_stripped)
        }
    }
}

/// Simple version string comparison
fn compare_version_strings(a: &str, b: &str) -> std::cmp::Ordering {
    let parse_parts = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter_map(|p| p.parse().ok())
            .collect()
    };

    let parts_a = parse_parts(a);
    let parts_b = parse_parts(b);

    for (pa, pb) in parts_a.iter().zip(parts_b.iter()) {
        match pa.cmp(pb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    parts_a.len().cmp(&parts_b.len())
}

/// Normalize repository URL
fn normalize_repo_url(url: &str) -> String {
    let url = url
        .strip_prefix("git+")
        .unwrap_or(url)
        .strip_suffix(".git")
        .unwrap_or(url);

    if url.starts_with("git://") {
        url.replace("git://", "https://")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dev_version() {
        assert!(is_dev_version("dev-master"));
        assert!(is_dev_version("dev-main"));
        assert!(is_dev_version("1.0.x-dev"));
        assert!(!is_dev_version("1.0.0"));
        assert!(!is_dev_version("v2.3.4"));
    }

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0-alpha"));
        assert!(is_prerelease("1.0.0-beta.1"));
        assert!(is_prerelease("1.0.0-RC1"));
        assert!(is_prerelease("dev-master"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("v2.3.4"));
    }

    #[test]
    fn test_compare_packagist_versions() {
        use std::cmp::Ordering;

        assert_eq!(compare_packagist_versions("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(
            compare_packagist_versions("v2.0.0", "v1.0.0"),
            Ordering::Greater
        );
        assert_eq!(
            compare_packagist_versions("1.0.0", "1.0.0"),
            Ordering::Equal
        );
    }

    #[test]
    fn test_normalize_repo_url() {
        assert_eq!(
            normalize_repo_url("git+https://github.com/user/repo.git"),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("git://github.com/user/repo"),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo"),
            "https://github.com/user/repo"
        );
    }
}
