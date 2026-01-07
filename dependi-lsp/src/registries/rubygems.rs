//! Client for RubyGems.org registry
//!
//! Provides package metadata and version information for Ruby gems.
//! API documentation: https://guides.rubygems.org/rubygems-org-api/

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::{Registry, VersionInfo};

/// Client for the RubyGems.org registry
pub struct RubyGemsRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl RubyGemsRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mpiton/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client: Arc::new(client),
            base_url: "https://rubygems.org/api/v1".to_string(),
        })
    }
}

impl Default for RubyGemsRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create RubyGemsRegistry")
    }
}

/// RubyGems API response for a gem
#[derive(Debug, Deserialize)]
struct GemResponse {
    #[serde(rename = "name")]
    _name: String,
    version: String,
    info: Option<String>,
    licenses: Option<Vec<String>>,
    homepage_uri: Option<String>,
    source_code_uri: Option<String>,
    project_uri: Option<String>,
}

/// RubyGems API response for gem versions
#[derive(Debug, Deserialize)]
struct GemVersionResponse {
    number: String,
    #[serde(default)]
    _prerelease: bool,
}

impl Registry for RubyGemsRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Fetch gem info
        let gem_url = format!("{}/gems/{}.json", self.base_url, package_name);
        let gem_response = self.client.get(&gem_url).send().await?;

        if !gem_response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch gem info for {}: {}",
                package_name,
                gem_response.status()
            );
        }

        let gem: GemResponse = gem_response.json().await?;

        // Fetch all versions
        let versions_url = format!("{}/versions/{}.json", self.base_url, package_name);
        let versions_response = self.client.get(&versions_url).send().await?;

        let versions: Vec<String> = if versions_response.status().is_success() {
            let version_list: Vec<GemVersionResponse> = versions_response.json().await?;
            version_list.iter().map(|v| v.number.clone()).collect()
        } else {
            vec![gem.version.clone()]
        };

        // Find latest stable version (non-prerelease)
        let latest_stable = versions
            .iter()
            .find(|v| !is_prerelease(v))
            .cloned()
            .or_else(|| Some(gem.version.clone()));

        // Find latest prerelease
        let latest_prerelease = versions.iter().find(|v| is_prerelease(v)).cloned();

        // Get license (first one if multiple)
        let license = gem.licenses.and_then(|l| l.into_iter().next());

        // Get repository URL (prefer source_code_uri, fallback to homepage)
        let repository = gem.source_code_uri.or_else(|| gem.homepage_uri.clone());

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description: gem.info,
            homepage: gem.homepage_uri.or(gem.project_uri),
            repository,
            license,
            vulnerabilities: vec![], // Will be filled by OSV
            deprecated: false,       // RubyGems doesn't have a deprecation flag in API
            yanked: false,           // Will check individual versions if needed
        })
    }
}

/// Check if a version is a prerelease
fn is_prerelease(version: &str) -> bool {
    version.contains(".pre")
        || version.contains(".alpha")
        || version.contains(".beta")
        || version.contains(".rc")
        || version.contains("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0.pre"));
        assert!(is_prerelease("1.0.0.alpha"));
        assert!(is_prerelease("1.0.0.beta.1"));
        assert!(is_prerelease("1.0.0.rc1"));
        assert!(is_prerelease("1.0.0-pre"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.0.0"));
        assert!(!is_prerelease("1.0.0.1"));
    }
}
