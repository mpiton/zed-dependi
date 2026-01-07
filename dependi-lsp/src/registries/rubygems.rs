//! Client for RubyGems.org registry
//!
//! Provides package metadata and version information for Ruby gems.
//! API documentation: https://guides.rubygems.org/rubygems-org-api/

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::{Registry, VersionInfo};

/// Client for the RubyGems.org registry
pub struct RubyGemsRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl RubyGemsRegistry {
    /// Creates a RubyGemsRegistry that uses the provided shared HTTP client.
    ///
    /// The provided `client` will be used for all HTTP requests to the RubyGems API. The registry's
    /// base API URL is set to "https://rubygems.org/api/v1".
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use reqwest::Client;
    /// use dependi_lsp::registries::rubygems::RubyGemsRegistry;
    ///
    /// let client = Arc::new(Client::new());
    /// let _registry = RubyGemsRegistry::with_client(client);
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://rubygems.org/api/v1".to_string(),
        }
    }
}

impl Default for RubyGemsRegistry {
    /// Creates a `RubyGemsRegistry` configured with the shared HTTP client used by the module.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = RubyGemsRegistry::default();
    /// // `registry` is ready to query the RubyGems API.
    /// let _ = registry;
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
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
    version_created_at: Option<String>,
}

/// RubyGems API response for version list
#[derive(Debug, Deserialize)]
struct VersionResponse {
    number: String,
    created_at: Option<String>,
}

impl Registry for RubyGemsRegistry {
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

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

        // Fetch all versions with dates
        let versions_url = format!("{}/versions/{}.json", self.base_url, package_name);
        let (versions, release_dates) = match self.client.get(&versions_url).send().await {
            Ok(response) if response.status().is_success() => {
                let version_list: Vec<VersionResponse> = response.json().await.unwrap_or_default();
                let versions: Vec<String> = version_list.iter().map(|v| v.number.clone()).collect();
                let dates: HashMap<String, DateTime<Utc>> = version_list
                    .into_iter()
                    .filter_map(|v| {
                        v.created_at.as_ref().and_then(|time_str| {
                            DateTime::parse_from_rfc3339(time_str)
                                .ok()
                                .map(|dt| (v.number.clone(), dt.with_timezone(&Utc)))
                        })
                    })
                    .collect();
                (versions, dates)
            }
            _ => {
                // Fallback to just latest version with its date
                let mut dates = HashMap::new();
                if let Some(time_str) = &gem.version_created_at
                    && let Ok(dt) = DateTime::parse_from_rfc3339(time_str)
                {
                    dates.insert(gem.version.clone(), dt.with_timezone(&Utc));
                }
                (vec![gem.version.clone()], dates)
            }
        };

        // Use the latest version from gem info
        let latest_stable = Some(gem.version.clone());

        // Get license (first one if multiple)
        let license = gem.licenses.and_then(|l| l.into_iter().next());

        // Get repository URL (prefer source_code_uri, fallback to homepage)
        let repository = gem.source_code_uri.or_else(|| gem.homepage_uri.clone());

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease: None,
            versions,
            description: gem.info,
            homepage: gem.homepage_uri.or(gem.project_uri),
            repository,
            license,
            vulnerabilities: vec![], // Will be filled by OSV
            deprecated: false,       // RubyGems doesn't have a deprecation flag in API
            yanked: false,
            yanked_versions: vec![], // Not applicable to RubyGems
            release_dates,
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