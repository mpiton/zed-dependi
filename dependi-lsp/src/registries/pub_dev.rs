//! Client for pub.dev registry (Dart/Flutter packages)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::{Registry, VersionInfo};

/// Client for the pub.dev registry
pub struct PubDevRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl PubDevRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client: Arc::new(client),
            base_url: "https://pub.dev/api".to_string(),
        })
    }
}

impl Default for PubDevRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create PubDevRegistry")
    }
}

// pub.dev API response structures
#[derive(Debug, Deserialize)]
struct PubPackageResponse {
    #[serde(rename = "name")]
    _name: String,
    latest: PubVersionInfo,
    versions: Vec<PubVersionInfo>,
}

#[derive(Debug, Deserialize)]
struct PubVersionInfo {
    version: String,
    pubspec: PubPubspec,
    #[serde(default)]
    retracted: bool,
    published: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct PubPubspec {
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    #[serde(default)]
    discontinued: bool,
}

impl Registry for PubDevRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        let url = format!("{}/packages/{}", self.base_url, package_name);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch package info for {}: {}",
                package_name,
                response.status()
            );
        }

        let pkg: PubPackageResponse = response.json().await?;

        // Get all versions (not retracted)
        let versions: Vec<String> = pkg
            .versions
            .iter()
            .filter(|v| !v.retracted)
            .map(|v| v.version.clone())
            .collect();

        // Find latest stable version
        let latest_stable = versions
            .iter()
            .find(|v| !is_prerelease(v))
            .cloned()
            .or_else(|| Some(pkg.latest.version.clone()));

        // Find latest prerelease
        let latest_prerelease = versions.iter().find(|v| is_prerelease(v)).cloned();

        // Collect release dates
        let release_dates: HashMap<String, DateTime<Utc>> = pkg
            .versions
            .iter()
            .filter_map(|v| v.published.map(|p| (v.version.clone(), p)))
            .collect();

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description: pkg.latest.pubspec.description,
            homepage: pkg.latest.pubspec.homepage,
            repository: pkg.latest.pubspec.repository,
            license: None,           // pub.dev doesn't expose license in API
            vulnerabilities: vec![], // Will be filled by OSV
            deprecated: pkg.latest.pubspec.discontinued,
            yanked: pkg.latest.retracted,
            yanked_versions: vec![], // Not applicable to pub.dev
            release_dates,
        })
    }
}

fn is_prerelease(version: &str) -> bool {
    version.contains('-')
        || version.contains("dev")
        || version.contains("alpha")
        || version.contains("beta")
        || version.contains("rc")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0-dev.1"));
        assert!(is_prerelease("1.0.0-alpha"));
        assert!(is_prerelease("1.0.0-beta.1"));
        assert!(is_prerelease("1.0.0-rc.1"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.0.0"));
    }
}
