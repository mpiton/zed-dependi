//! Registry clients for fetching package version information

use serde::{Deserialize, Serialize};

/// Information about a package version from a registry
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionInfo {
    /// Latest stable version
    pub latest: Option<String>,
    /// Latest prerelease version
    pub latest_prerelease: Option<String>,
    /// All available versions
    pub versions: Vec<String>,
    /// Package description
    pub description: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// Repository URL (GitHub, etc.)
    pub repository: Option<String>,
    /// SPDX license identifier
    pub license: Option<String>,
    /// Known vulnerabilities
    pub vulnerabilities: Vec<Vulnerability>,
    /// Whether the package is deprecated
    pub deprecated: bool,
    /// Whether the version is yanked (Rust specific)
    pub yanked: bool,
}

/// Vulnerability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    /// CVE or advisory ID
    pub id: String,
    /// Severity level
    pub severity: VulnerabilitySeverity,
    /// Description of the vulnerability
    pub description: String,
    /// URL for more information
    pub url: Option<String>,
}

/// Vulnerability severity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VulnerabilitySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Trait for registry clients
#[allow(async_fn_in_trait)]
pub trait Registry: Send + Sync {
    /// Get version information for a package
    async fn get_version_info(&self, package_name: &str) -> anyhow::Result<VersionInfo>;

    /// Get the latest version of a package
    async fn get_latest_version(&self, package_name: &str) -> anyhow::Result<Option<String>> {
        Ok(self.get_version_info(package_name).await?.latest)
    }
}

pub mod crates_io;
pub mod npm;
pub mod pypi;
pub mod go_proxy;
pub mod packagist;

// TODO: Implement additional registry clients
// pub mod pub_dev;
// pub mod nuget;
