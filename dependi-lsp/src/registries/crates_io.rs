//! Client for crates.io registry

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{Registry, VersionInfo};

/// Rate limiter to respect crates.io's 1 request/second limit
struct RateLimiter {
    last_request: Instant,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(requests_per_second: f64) -> Self {
        Self {
            last_request: Instant::now() - Duration::from_secs(10),
            min_interval: Duration::from_secs_f64(1.0 / requests_per_second),
        }
    }

    async fn wait(&mut self) {
        let elapsed = self.last_request.elapsed();
        if elapsed < self.min_interval {
            tokio::time::sleep(self.min_interval - elapsed).await;
        }
        self.last_request = Instant::now();
    }
}

/// Client for the crates.io registry
pub struct CratesIoRegistry {
    client: Client,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    base_url: String,
}

impl CratesIoRegistry {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(1.0))),
            base_url: "https://crates.io/api/v1".to_string(),
        })
    }
}

impl Default for CratesIoRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create CratesIoRegistry")
    }
}

// API response structures
#[derive(Debug, Deserialize)]
struct CrateResponse {
    #[serde(rename = "crate")]
    crate_info: CrateInfo,
    versions: Vec<VersionEntry>,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
    max_stable_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VersionEntry {
    num: String,
    yanked: bool,
    license: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

impl Registry for CratesIoRegistry {
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo> {
        // Rate limiting
        {
            let mut limiter = self.rate_limiter.lock().await;
            limiter.wait().await;
        }

        let url = format!("{}/crates/{}", self.base_url, package_name);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch crate info for {}: {}",
                package_name,
                response.status()
            );
        }

        let crate_response: CrateResponse = response.json().await?;

        // Find latest stable version (not yanked, no prerelease)
        let latest_stable = crate_response
            .crate_info
            .max_stable_version
            .clone()
            .or_else(|| {
                crate_response
                    .versions
                    .iter()
                    .find(|v| !v.yanked && !is_prerelease(&v.num))
                    .map(|v| v.num.clone())
            });

        // Find latest prerelease
        let latest_prerelease = crate_response
            .versions
            .iter()
            .find(|v| !v.yanked && is_prerelease(&v.num))
            .map(|v| v.num.clone());

        // Get all versions (not yanked)
        let versions: Vec<String> = crate_response
            .versions
            .iter()
            .filter(|v| !v.yanked)
            .map(|v| v.num.clone())
            .collect();

        // Get license from latest version
        let license = crate_response
            .versions
            .first()
            .and_then(|v| v.license.clone());

        // Collect all yanked versions
        let yanked_versions: Vec<String> = crate_response
            .versions
            .iter()
            .filter(|v| v.yanked)
            .map(|v| v.num.clone())
            .collect();

        // Collect release dates for all versions
        let release_dates: HashMap<String, DateTime<Utc>> = crate_response
            .versions
            .iter()
            .filter_map(|v| v.created_at.map(|dt| (v.num.clone(), dt)))
            .collect();

        // Check if latest version is yanked (kept for backwards compatibility)
        let yanked = crate_response.versions.first().is_some_and(|v| v.yanked);

        Ok(VersionInfo {
            latest: latest_stable,
            latest_prerelease,
            versions,
            description: crate_response.crate_info.description,
            homepage: crate_response.crate_info.homepage,
            repository: crate_response.crate_info.repository,
            license,
            vulnerabilities: vec![], // Filled by OSV
            deprecated: false,       // Filled by OSV
            yanked,
            yanked_versions,
            release_dates,
        })
    }
}

fn is_prerelease(version: &str) -> bool {
    version.contains('-')
        || version.contains("alpha")
        || version.contains("beta")
        || version.contains("rc")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease("1.0.0-alpha"));
        assert!(is_prerelease("1.0.0-beta.1"));
        assert!(is_prerelease("1.0.0-rc1"));
        assert!(!is_prerelease("1.0.0"));
        assert!(!is_prerelease("2.3.4"));
    }
}
