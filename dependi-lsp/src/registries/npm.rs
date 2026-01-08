//! # npm Registry Client
//!
//! This module implements a client for the [npm](https://www.npmjs.com) registry,
//! the default package registry for Node.js and JavaScript packages.
//!
//! ## API Details
//!
//! - **Base URL**: `https://registry.npmjs.org`
//! - **API Version**: Registry API (stable)
//! - **Authentication**: Bearer token from `.npmrc` (optional)
//! - **CORS**: Enabled for browser-based access
//!
//! ## Rate Limiting
//!
//! npm does **not** enforce hard rate limits on read operations, but implements:
//!
//! - **IP-based blocking**: For abusive behavior patterns
//! - **Cloudflare protection**: May trigger CAPTCHA for suspicious traffic
//! - **Best practice**: Keep requests under 100/minute for bulk operations
//!
//! ## API Endpoints Used
//!
//! ### Fetch Package Info
//!
//! - **Endpoint**: `GET /{package-name}`
//! - **Scoped packages**: `GET /@scope%2fpackage-name` (URL encoded `/`)
//! - **Response**: JSON with full package metadata
//! - **Fields**:
//!   - `dist-tags.latest`: Current stable version
//!   - `dist-tags.next`: Latest prerelease (if exists)
//!   - `versions{}`: Map of version string to version metadata
//!   - `time{}`: Map of version string to publish timestamp
//!
//! ## Response Parsing
//!
//! - **Version format**: Semver with optional prerelease (`-alpha`, `-beta`, `-canary`)
//! - **Date format**: RFC 3339 (`2024-01-15T10:30:00.000Z`)
//! - **Deprecated packages**: `deprecated` field in version metadata (string message)
//! - **License**: Can be string or object with `type` field
//! - **Repository**: Can be string or object with `url` field
//!
//! ## Edge Cases and Quirks
//!
//! - **Scoped packages**: Must URL-encode the slash (`@scope/pkg` → `@scope%2fpkg`)
//! - **Repository URL formats**: May include `git+https://`, `git://`, or `.git` suffix
//! - **Large packages**: May have thousands of versions (e.g., lodash)
//! - **Unpublished packages**: Return 404 but may have been available previously
//! - **Private packages**: Require authentication; return 401/403 without token
//! - **Engines field**: Contains Node.js version constraints (not exposed by this client)
//!
//! ## Error Handling
//!
//! - **Network errors**: Returned as `anyhow::Error`
//! - **API errors**: 404 for not found, 401/403 for auth issues
//! - **Timeouts**: 10 second default timeout
//!
//! ## External References
//!
//! - [npm Registry API](https://github.com/npm/registry/blob/main/docs/REGISTRY-API.md)
//! - [Package Metadata Specification](https://github.com/npm/registry/blob/main/docs/responses/package-metadata.md)
//! - [npm CLI Documentation](https://docs.npmjs.com/cli)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

use super::http_client::create_shared_client;
use super::version_utils::is_prerelease_npm;
use super::{Registry, VersionInfo};

/// Client for the npm registry
pub struct NpmRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl NpmRegistry {
    /// Constructs an NpmRegistry that uses the provided shared HTTP client and the default npm registry base URL.
    ///
    /// The supplied `client` is used for all HTTP requests performed by the registry; the base URL is set to
    /// `https://registry.npmjs.org`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use dependi_lsp::registries::npm::NpmRegistry;
    ///
    /// let client = Arc::new(reqwest::Client::new());
    /// let registry = NpmRegistry::with_client(client);
    /// ```
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://registry.npmjs.org".to_string(),
        }
    }
}

impl Default for NpmRegistry {
    /// Creates a default NpmRegistry configured with a shared HTTP client and the standard npm registry base URL.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use dependi_lsp::registries::npm::NpmRegistry;
    ///
    /// let registry = NpmRegistry::default();
    /// ```
    fn default() -> Self {
        Self::with_client(create_shared_client().expect("Failed to create HTTP client"))
    }
}

// API response structures
#[derive(Debug, Deserialize)]
struct PackageResponse {
    description: Option<String>,
    homepage: Option<String>,
    repository: Option<Repository>,
    license: Option<LicenseField>,
    #[serde(rename = "dist-tags")]
    dist_tags: Option<DistTags>,
    versions: Option<HashMap<String, VersionMetadata>>,
    time: Option<HashMap<String, String>>,
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
    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }

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
            .or_else(|| versions.iter().find(|v| is_prerelease_npm(v)).cloned());

        // Check if latest version is deprecated
        let deprecated = pkg
            .versions
            .as_ref()
            .and_then(|v| latest.as_ref().and_then(|l| v.get(l)))
            .is_some_and(|m| m.deprecated.is_some());

        // Get repository URL
        let repository = pkg.repository.as_ref().and_then(|r| r.url());

        // Parse release dates from the time field (excluding "created" and "modified" keys)
        let release_dates: HashMap<String, DateTime<Utc>> = pkg
            .time
            .as_ref()
            .map(|time_map| {
                time_map
                    .iter()
                    .filter(|(k, _)| *k != "created" && *k != "modified")
                    .filter_map(|(version, date_str)| {
                        DateTime::parse_from_rfc3339(date_str)
                            .ok()
                            .map(|dt| (version.clone(), dt.with_timezone(&Utc)))
                    })
                    .collect()
            })
            .unwrap_or_default();

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
            yanked_versions: vec![], // Not applicable to npm
            release_dates,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prerelease() {
        assert!(is_prerelease_npm("1.0.0-alpha"));
        assert!(is_prerelease_npm("1.0.0-beta.1"));
        assert!(is_prerelease_npm("1.0.0-rc.1"));
        assert!(is_prerelease_npm("18.3.0-canary"));
        assert!(!is_prerelease_npm("1.0.0"));
        assert!(!is_prerelease_npm("2.3.4"));
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

    #[test]
    fn test_repository_string_variant() {
        let repo = Repository::String("git+https://github.com/user/repo.git".to_string());
        assert_eq!(repo.url(), Some("https://github.com/user/repo".to_string()));
    }

    #[test]
    fn test_repository_object_variant() {
        let repo = Repository::Object {
            url: Some("git://github.com/user/repo".to_string()),
        };
        assert_eq!(repo.url(), Some("https://github.com/user/repo".to_string()));
    }

    #[test]
    fn test_repository_object_none() {
        let repo = Repository::Object { url: None };
        assert_eq!(repo.url(), None);
    }

    #[test]
    fn test_license_field_string() {
        let license = LicenseField::String("MIT".to_string());
        assert_eq!(license.as_string(), Some("MIT".to_string()));
    }

    #[test]
    fn test_license_field_object() {
        let license = LicenseField::Object {
            r#type: Some("Apache-2.0".to_string()),
        };
        assert_eq!(license.as_string(), Some("Apache-2.0".to_string()));
    }

    #[test]
    fn test_license_field_object_none() {
        let license = LicenseField::Object { r#type: None };
        assert_eq!(license.as_string(), None);
    }

    #[test]
    fn test_scoped_package_url_encoding() {
        let package_name = "@types/node";
        let encoded = if package_name.starts_with('@') {
            package_name.replace('/', "%2f")
        } else {
            package_name.to_string()
        };
        assert_eq!(encoded, "@types%2fnode");
    }

    #[test]
    fn test_normal_package_no_encoding() {
        let package_name = "lodash";
        let encoded = if package_name.starts_with('@') {
            package_name.replace('/', "%2f")
        } else {
            package_name.to_string()
        };
        assert_eq!(encoded, "lodash");
    }
}
