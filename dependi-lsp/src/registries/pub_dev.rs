//! Client for pub.dev registry (Dart/Flutter packages)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::{Registry, VersionInfo};

/// Client for the pub.dev registry
pub struct PubDevRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl PubDevRegistry {
    /// Creates a `PubDevRegistry` configured to use the given shared HTTP client.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use dependi_lsp::registries::pub_dev::PubDevRegistry;
    ///
    /// let client = Arc::new(reqwest::Client::new());
    /// let _registry = PubDevRegistry::with_client(client);
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://pub.dev/api".to_string(),
        }
    }
}

impl Default for PubDevRegistry {
    /// Constructs a PubDevRegistry using a shared HTTP client created by `create_shared_client`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use dependi_lsp::registries::pub_dev::PubDevRegistry;
    ///
    /// let registry = PubDevRegistry::default();
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
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
    published: Option<String>,
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
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

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
            .filter_map(|v| {
                v.published.as_ref().and_then(|time_str| {
                    DateTime::parse_from_rfc3339(time_str)
                        .ok()
                        .map(|dt| (v.version.clone(), dt.with_timezone(&Utc)))
                })
            })
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
