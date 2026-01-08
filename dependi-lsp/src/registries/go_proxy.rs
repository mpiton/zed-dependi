//! Client for Go module proxy (proxy.golang.org)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::version_utils::is_prerelease_go;
use super::{Registry, VersionInfo};

/// Client for the Go module proxy
pub struct GoProxyRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl GoProxyRegistry {
    /// Creates a GoProxyRegistry that uses the provided shared HTTP client and the default Go proxy base URL.
    ///
    /// `client` is the shared `reqwest::Client` used for all outgoing HTTP requests to the Go proxy.
    /// The returned registry is configured with `base_url` set to "https://proxy.golang.org".
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use dependi_lsp::registries::go_proxy::GoProxyRegistry;
    ///
    /// let client = Arc::new(reqwest::Client::new());
    /// let _registry = GoProxyRegistry::with_client(client);
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://proxy.golang.org".to_string(),
        }
    }
}

impl Default for GoProxyRegistry {
    /// Creates a GoProxyRegistry configured with a shared HTTP client.
    ///
    /// The registry's HTTP client is produced by `create_shared_client`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use dependi_lsp::registries::go_proxy::GoProxyRegistry;
    ///
    /// let registry = GoProxyRegistry::default();
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
    }
}

// Go proxy API response for version info
#[derive(Debug, Deserialize)]
struct VersionInfoResponse {
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "Time")]
    time: Option<String>,
}

impl Registry for GoProxyRegistry {
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

    async fn get_version_info(&self, module_path: &str) -> anyhow::Result<VersionInfo> {
        // Encode module path for URL
        // Go proxy requires case-encoding: uppercase letters become ! followed by lowercase
        let encoded_path = encode_module_path(module_path);

        // Fetch list of versions
        let versions = self.fetch_versions(&encoded_path).await.unwrap_or_default();

        // Fetch latest version info
        let latest = self.fetch_latest(&encoded_path).await.ok();

        // Sort versions in descending order
        let mut sorted_versions = versions.clone();
        sorted_versions.sort_by(|a, b| compare_go_versions(b, a));

        // Find latest stable version (no prerelease suffix)
        let latest_stable = latest.as_ref().map(|l| l.version.clone()).or_else(|| {
            sorted_versions
                .iter()
                .find(|v| !is_prerelease_go(v))
                .cloned()
        });

        // Find latest prerelease
        let latest_prerelease = sorted_versions
            .iter()
            .find(|v| is_prerelease_go(v))
            .cloned();

        // Build repository URL for common hosts
        let repository =
            if module_path.starts_with("github.com/") || module_path.starts_with("gitlab.com/") {
                Some(format!("https://{}", module_path))
            } else {
                None
            };

        // Fetch release dates for versions (fetch info for each version in parallel)
        let release_dates = self
            .fetch_version_times(&encoded_path, &sorted_versions)
            .await;

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions: sorted_versions,
            description: None, // Go proxy doesn't provide descriptions
            homepage: None,
            repository,
            license: None,           // Would need to fetch go.mod or LICENSE file
            vulnerabilities: vec![], // TODO: Integrate vuln.go.dev
            deprecated: false,
            yanked: false,
            yanked_versions: vec![], // Not applicable to Go
            release_dates,
        })
    }
}

impl GoProxyRegistry {
    /// Fetch list of available versions
    async fn fetch_versions(&self, encoded_path: &str) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/{}/@v/list", self.base_url, encoded_path);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch versions for {}: {}",
                encoded_path,
                response.status()
            );
        }

        let text = response.text().await?;
        let versions: Vec<String> = text.lines().map(|s| s.trim().to_string()).collect();

        Ok(versions)
    }

    /// Fetch latest version info
    async fn fetch_latest(&self, encoded_path: &str) -> anyhow::Result<VersionInfoResponse> {
        let url = format!("{}/{}/@latest", self.base_url, encoded_path);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch latest for {}: {}",
                encoded_path,
                response.status()
            );
        }

        let info: VersionInfoResponse = response.json().await?;
        Ok(info)
    }

    /// Fetch version info for a specific version
    async fn fetch_version_info(
        &self,
        encoded_path: &str,
        version: &str,
    ) -> Option<VersionInfoResponse> {
        let url = format!("{}/{}/@v/{}.info", self.base_url, encoded_path, version);

        let response = self.client.get(&url).send().await.ok()?;

        if !response.status().is_success() {
            return None;
        }

        response.json().await.ok()
    }

    /// Fetch release times for a list of versions (limited to first 10 for performance)
    async fn fetch_version_times(
        &self,
        encoded_path: &str,
        versions: &[String],
    ) -> HashMap<String, DateTime<Utc>> {
        use futures::future::join_all;

        let futures: Vec<_> = versions
            .iter()
            .take(10)
            .map(|v| async move {
                self.fetch_version_info(encoded_path, v)
                    .await
                    .and_then(|info| {
                        info.time.as_ref().and_then(|time_str| {
                            DateTime::parse_from_rfc3339(time_str)
                                .ok()
                                .map(|dt| (v.clone(), dt.with_timezone(&Utc)))
                        })
                    })
            })
            .collect();

        let results = join_all(futures).await;
        results.into_iter().flatten().collect()
    }
}

/// Encode module path for Go proxy URL
/// Uppercase letters are replaced with ! followed by lowercase
fn encode_module_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len() * 2);

    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            result.push('!');
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }

    result
}

/// Compare Go versions for sorting
fn compare_go_versions(a: &str, b: &str) -> std::cmp::Ordering {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_module_path() {
        assert_eq!(
            encode_module_path("github.com/Azure/azure-sdk-for-go"),
            "github.com/!azure/azure-sdk-for-go"
        );
        assert_eq!(
            encode_module_path("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
        assert_eq!(encode_module_path("golang.org/x/text"), "golang.org/x/text");
    }

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease_go("v1.0.0-rc1"));
        assert!(is_prerelease_go("v2.0.0-beta.1"));
        assert!(is_prerelease_go("v3.0.0-alpha"));
        assert!(!is_prerelease_go("v1.0.0"));
        assert!(!is_prerelease_go("v2.3.4"));
    }

    #[test]
    fn test_compare_go_versions() {
        use std::cmp::Ordering;

        assert_eq!(compare_go_versions("v1.0.0", "v2.0.0"), Ordering::Less);
        assert_eq!(compare_go_versions("v2.0.0", "v1.0.0"), Ordering::Greater);
        assert_eq!(compare_go_versions("v1.0.0", "v1.0.0"), Ordering::Equal);
        assert_eq!(compare_go_versions("v1.10.0", "v1.9.0"), Ordering::Greater);
    }
}
