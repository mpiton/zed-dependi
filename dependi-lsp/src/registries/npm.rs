//! Client for npm registry

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::{Registry, VersionInfo};

/// Client for the npm registry
pub struct NpmRegistry {
    client: Client,
    base_url: String,
}

impl NpmRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            base_url: "https://registry.npmjs.org".to_string(),
        })
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self { client, base_url })
    }
}

impl Default for NpmRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create NpmRegistry")
    }
}

// API response structures
#[derive(Debug, Deserialize)]
struct PackageResponse {
    name: String,
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<Repository>,
    license: Option<LicenseField>,
    #[serde(rename = "dist-tags")]
    dist_tags: Option<DistTags>,
    versions: Option<HashMap<String, VersionMetadata>>,
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
