//! Client for PyPI (Python Package Index) registry

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::{Registry, VersionInfo};

/// Client for the PyPI registry
pub struct PyPiRegistry {
    client: Client,
    base_url: String,
}

impl PyPiRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            base_url: "https://pypi.org/pypi".to_string(),
        })
    }
}

impl Default for PyPiRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create PyPiRegistry")
    }
}

// PyPI API response structures
#[derive(Debug, Deserialize)]
struct PyPiResponse {
    info: PackageInfo,
    releases: HashMap<String, Vec<ReleaseFile>>,
}

#[derive(Debug, Deserialize)]
struct PackageInfo {
    /// Latest version
    version: String,
    /// Package description
    summary: Option<String>,
    /// Homepage URL
    home_page: Option<String>,
    /// License string
    license: Option<String>,
    /// Project URLs (may contain Repository, Homepage, etc.)
    project_urls: Option<HashMap<String, String>>,
    /// Classifiers (can be used to detect deprecated packages)
    classifiers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ReleaseFile {
    /// Whether this file has been yanked
    yanked: Option<bool>,
    /// Upload time for this file (ISO 8601 format without timezone)
    upload_time: Option<String>,
}

impl Registry for PyPiRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Normalize package name (PyPI is case-insensitive, uses lowercase)
        let normalized_name = normalize_package_name(package_name);

        let url = format!("{}/{}/json", self.base_url, normalized_name);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch package info for {}: {}",
                package_name,
                response.status()
            );
        }

        let pypi_response: PyPiResponse = response.json().await?;

        // Get all versions sorted by semver (newest first)
        let mut versions: Vec<String> = pypi_response
            .releases
            .iter()
            .filter(|(_, files)| {
                // Filter out yanked versions
                !files.iter().any(|f| f.yanked.unwrap_or(false))
            })
            .map(|(version, _)| version.clone())
            .collect();

        // Sort versions in descending order
        versions.sort_by(|a, b| compare_python_versions(b, a));

        // Find latest stable version (non-prerelease)
        let latest_stable = versions
            .iter()
            .find(|v| !is_prerelease(v))
            .cloned()
            .or_else(|| Some(pypi_response.info.version.clone()));

        // Find latest prerelease
        let latest_prerelease = versions.iter().find(|v| is_prerelease(v)).cloned();

        // Extract repository URL from project_urls
        let repository = pypi_response.info.project_urls.as_ref().and_then(|urls| {
            urls.get("Repository")
                .or_else(|| urls.get("Source"))
                .or_else(|| urls.get("Source Code"))
                .or_else(|| urls.get("GitHub"))
                .cloned()
        });

        // Extract homepage
        let homepage = pypi_response.info.home_page.clone().or_else(|| {
            pypi_response
                .info
                .project_urls
                .as_ref()
                .and_then(|urls| urls.get("Homepage").cloned())
        });

        // Check if deprecated (via classifiers)
        let deprecated = pypi_response
            .info
            .classifiers
            .as_ref()
            .is_some_and(|classifiers| {
                classifiers
                    .iter()
                    .any(|c| c.contains("Development Status :: 7 - Inactive"))
            });

        // Extract release dates from releases (use the first file's upload_time for each version)
        let release_dates: HashMap<String, DateTime<Utc>> = pypi_response
            .releases
            .iter()
            .filter_map(|(version, files)| {
                files
                    .first()
                    .and_then(|f| f.upload_time.as_ref())
                    .and_then(|time_str| parse_pypi_datetime(time_str))
                    .map(|dt| (version.clone(), dt))
            })
            .collect();

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description: pypi_response.info.summary,
            homepage,
            repository,
            license: pypi_response.info.license,
            vulnerabilities: vec![], // TODO: Integrate Safety/OSV
            deprecated,
            yanked: false,
            yanked_versions: vec![], // Not applicable to PyPI
            release_dates,
        })
    }
}

/// Parse PyPI datetime format (ISO 8601 without timezone, assumed UTC)
fn parse_pypi_datetime(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|naive| naive.and_utc())
}

/// Normalize Python package name according to PEP 503
/// - Lowercase
/// - Replace underscores and dots with hyphens
fn normalize_package_name(name: &str) -> String {
    name.to_lowercase().replace(['_', '.'], "-")
}

/// Check if a version is a prerelease
fn is_prerelease(version: &str) -> bool {
    let v = version.to_lowercase();
    v.contains("dev")
        || v.contains("alpha")
        || v.contains("beta")
        || v.contains("rc")
        || v.contains('a') && v.chars().last().is_some_and(|c| c.is_ascii_digit())
        || v.contains('b') && v.chars().last().is_some_and(|c| c.is_ascii_digit())
        || v.contains(".dev")
        || v.contains(".post") // post-releases are actually stable, but let's include for completeness
}

/// Compare Python versions for sorting
/// Returns Ordering for descending sort (newer versions first)
fn compare_python_versions(a: &str, b: &str) -> std::cmp::Ordering {
    // Try parsing as semver first
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(va), Ok(vb)) => va.cmp(&vb),
        _ => {
            // Fallback to simple string comparison with version-aware logic
            compare_version_strings(a, b)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_package_name() {
        assert_eq!(normalize_package_name("Flask"), "flask");
        assert_eq!(normalize_package_name("ruamel.yaml"), "ruamel-yaml");
        assert_eq!(
            normalize_package_name("typing_extensions"),
            "typing-extensions"
        );
        assert_eq!(normalize_package_name("Pillow"), "pillow");
    }

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0a1"));
        assert!(is_prerelease("1.0.0b2"));
        assert!(is_prerelease("1.0.0rc1"));
        assert!(is_prerelease("1.0.0.dev1"));
        assert!(is_prerelease("2.0.0alpha"));
        assert!(is_prerelease("2.0.0beta"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.3.4"));
    }

    #[test]
    fn test_compare_python_versions() {
        use std::cmp::Ordering;

        assert_eq!(compare_python_versions("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(compare_python_versions("2.0.0", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_python_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(
            compare_python_versions("1.10.0", "1.9.0"),
            Ordering::Greater
        );
    }
}
