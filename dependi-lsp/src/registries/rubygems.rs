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

impl Registry for RubyGemsRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Fetch gem info (contains latest version)
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

        // Use the latest version from gem info (skip fetching all versions for speed)
        let latest_stable = Some(gem.version.clone());

        // Get license (first one if multiple)
        let license = gem.licenses.and_then(|l| l.into_iter().next());

        // Get repository URL (prefer source_code_uri, fallback to homepage)
        let repository = gem.source_code_uri.or_else(|| gem.homepage_uri.clone());

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease: None,
            versions: vec![gem.version], // Only include latest for now
            description: gem.info,
            homepage: gem.homepage_uri.or(gem.project_uri),
            repository,
            license,
            vulnerabilities: vec![], // Will be filled by OSV
            deprecated: false,       // RubyGems doesn't have a deprecation flag in API
            yanked: false,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_rubygems_url_format() {
        let base_url = "https://rubygems.org/api/v1";
        let name = "rails";
        let url = format!("{}/gems/{}.json", base_url, name);
        assert_eq!(url, "https://rubygems.org/api/v1/gems/rails.json");
    }
}
