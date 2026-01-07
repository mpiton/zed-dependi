//! Client for npm registry

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::{Registry, VersionInfo};

/// Client for the npm registry
pub struct NpmRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl NpmRegistry {
    /// Constructs an NpmRegistry that uses the provided shared HTTP client and the default npm registry base URL.
    ///
    /// The supplied `client` is used for all HTTP requests performed by the registry; the base URL is set to
    /// "https://registry.npmjs.org".
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// // assume Client and NpmRegistry are in scope
    /// let client = Arc::new(Client::new());
    /// let registry = NpmRegistry::with_client(client.clone());
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://registry.npmjs.org".to_string(),
        }
    }
}

impl Default for NpmRegistry {
    /// Creates a default NpmRegistry configured with a shared HTTP client and the standard npm registry base URL.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = NpmRegistry::default();
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
    }
}

// API response structures
#[derive(Debug, Deserialize)]
struct PackageResponse {
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<Repository>,
    license: Option<LicenseField>,
    #[serde(rename = "dist-tags")]
    dist_tags: Option<DistTags>,
    versions: Option<HashMap<String, VersionMetadata>>,
    time: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Repository {
    String(String),
    Object { url: Option<String> },
}

impl Repository {
    fn url(&self) -> Option<String> {
        match self {
            Repository::String(s) => Some(normalize_repo_url(s)),
            Repository::Object { url } => url.as_ref().map(|u| normalize_repo_url(u)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LicenseField {
    String(String),
    Object { r#type: Option<String> },
}

impl LicenseField {
    fn as_string(&self) -> Option<String> {
        match self {
            LicenseField::String(s) => Some(s.clone()),
            LicenseField::Object { r#type } => r#type.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DistTags {
    latest: Option<String>,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VersionMetadata {
    deprecated: Option<String>,
}

fn normalize_repo_url(url: &str) -> String {
    // Convert git+https://github.com/user/repo.git to https://github.com/user/repo
    let url = url
        .strip_prefix("git+")
        .unwrap_or(url)
        .strip_suffix(".git")
        .unwrap_or(url);

    // Convert git://github.com to https://github.com
    if url.starts_with("git://") {
        return url.replace("git://", "https://");
    }

    url.to_string()
}

impl Registry for NpmRegistry {
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Handle scoped packages (@scope/name -> @scope%2fname)
        let encoded_name = if package_name.starts_with('@') {
            package_name.replace('/', "%2f")
        } else {
            package_name.to_string()
        };

        let url = format!("{}/{}", self.base_url, encoded_name);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch package info for {}: {}",
                package_name,
                response.status()
            );
        }

        let pkg: PackageResponse = response.json().await?;

        // Get latest version from dist-tags
        let latest = pkg.dist_tags.as_ref().and_then(|t| t.latest.clone());

        // Get all versions
        let versions: Vec<String> = pkg
            .versions
            .as_ref()
            .map(|v| {
                let mut versions: Vec<String> = v.keys().cloned().collect();
                // Sort versions in descending order (newest first)
                versions.sort_by(|a, b| {
                    match (semver::Version::parse(a), semver::Version::parse(b)) {
                        (Ok(va), Ok(vb)) => vb.cmp(&va),
                        _ => b.cmp(a),
                    }
                });
                versions
            })
            .unwrap_or_default();

        // Find latest prerelease
        let latest_prerelease = pkg
            .dist_tags
            .as_ref()
            .and_then(|t| t.next.clone())
            .or_else(|| versions.iter().find(|v| is_prerelease(v)).cloned());

        // Check if latest version is deprecated
        let deprecated = pkg
            .versions
            .as_ref()
            .and_then(|v| latest.as_ref().and_then(|l| v.get(l)))
            .is_some_and(|m| m.deprecated.is_some());

        // Get repository URL
        let repository = pkg.repository.as_ref().and_then(|r| r.url());

        // Parse release dates from the time field (excluding "created" and "modified" keys)
        let release_dates: HashMap<String, DateTime<Utc>> = pkg
            .time
            .as_ref()
            .map(|time_map| {
                time_map
                    .iter()
                    .filter(|(k, _)| *k != "created" && *k != "modified")
                    .filter_map(|(version, date_str)| {
                        DateTime::parse_from_rfc3339(date_str)
                            .ok()
                            .map(|dt| (version.clone(), dt.with_timezone(&Utc)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(VersionInfo {
            latest,
            latest_prerelease,
            versions,
            description: pkg.description,
            homepage: pkg.homepage,
            repository,
            license: pkg.license.and_then(|l| l.as_string()),
            vulnerabilities: vec![], // TODO: Integrate npm audit
            deprecated,
            yanked: false,
            yanked_versions: vec![], // Not applicable to npm
            release_dates,
        })
    }
}

fn is_prerelease(version: &str) -> bool {
    version.contains('-')
        || version.contains("alpha")
        || version.contains("beta")
        || version.contains("rc")
        || version.contains("canary")
        || version.contains("next")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0-alpha"));
        assert!(is_prerelease("1.0.0-beta.1"));
        assert!(is_prerelease("1.0.0-rc.1"));
        assert!(is_prerelease("18.3.0-canary"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.3.4"));
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
