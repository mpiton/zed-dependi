//! Client for Go module proxy (proxy.golang.org)

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::{Registry, VersionInfo};

/// Client for the Go module proxy
pub struct GoProxyRegistry {
    client: Client,
    base_url: String,
}

impl GoProxyRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            base_url: "https://proxy.golang.org".to_string(),
        })
    }
}

impl Default for GoProxyRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create GoProxyRegistry")
    }
}

// Go proxy API response for @latest
#[derive(Debug, Deserialize)]
struct LatestInfo {
    #[serde(rename = "Version")]
    version: String,
}

impl Registry for GoProxyRegistry {
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
        let latest_stable = latest
            .as_ref()
            .map(|l| l.version.clone())
            .or_else(|| sorted_versions.iter().find(|v| !is_prerelease(v)).cloned());

        // Find latest prerelease
        let latest_prerelease = sorted_versions.iter().find(|v| is_prerelease(v)).cloned();

        // Build repository URL for common hosts
        let repository =
            if module_path.starts_with("github.com/") || module_path.starts_with("gitlab.com/") {
                Some(format!("https://{}", module_path))
            } else {
                None
            };

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
    async fn fetch_latest(&self, encoded_path: &str) -> anyhow::Result<LatestInfo> {
        let url = format!("{}/{}/@latest", self.base_url, encoded_path);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch latest for {}: {}",
                encoded_path,
                response.status()
            );
        }

        let info: LatestInfo = response.json().await?;
        Ok(info)
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

/// Check if a version is a prerelease
fn is_prerelease(version: &str) -> bool {
    // Go uses semver-like versions with v prefix
    // Prereleases have -rc, -beta, -alpha, etc.
    version.contains("-rc")
        || version.contains("-alpha")
        || version.contains("-beta")
        || version.contains("-pre")
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
        assert!(is_prerelease("v1.0.0-rc1"));
        assert!(is_prerelease("v2.0.0-beta.1"));
        assert!(is_prerelease("v3.0.0-alpha"));
        assert!(!is_prerelease("v1.0.0"));
        assert!(!is_prerelease("v2.3.4"));
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
