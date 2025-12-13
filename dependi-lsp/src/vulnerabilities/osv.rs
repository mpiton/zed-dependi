//! OSV.dev API client for vulnerability data
//!
//! OSV (Open Source Vulnerabilities) provides a unified API for querying
//! vulnerability data across multiple ecosystems.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{VulnerabilityQuery, VulnerabilitySource};
use crate::registries::{Vulnerability, VulnerabilitySeverity};

const OSV_API_BASE: &str = "https://api.osv.dev/v1";

/// OSV.dev API client
pub struct OsvClient {
    client: Arc<Client>,
    base_url: String,
}

impl OsvClient {
    /// Create a new OSV client
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            client: Arc::new(client),
            base_url: OSV_API_BASE.to_string(),
        })
    }

    /// Create with custom base URL (for testing)
    #[cfg(test)]
    pub fn with_base_url(base_url: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp")
            .timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client: Arc::new(client),
            base_url,
        })
    }

    /// Convert OSV vulnerability to our Vulnerability struct
    fn convert_vulnerability(osv: &OsvVulnerability) -> Vulnerability {
        // Get CVE ID if available, otherwise use OSV ID
        let id = osv
            .aliases
            .as_ref()
            .and_then(|a| a.iter().find(|id| id.starts_with("CVE-")))
            .cloned()
            .unwrap_or_else(|| osv.id.clone());

        // Parse severity from CVSS score
        let severity = osv
            .severity
            .as_ref()
            .and_then(|s| s.first())
            .map(|s| parse_cvss_severity(&s.score))
            .unwrap_or(VulnerabilitySeverity::Medium);

        // Get description
        let description = osv
            .summary
            .clone()
            .or_else(|| osv.details.clone())
            .unwrap_or_else(|| format!("Vulnerability {}", osv.id));

        // Get advisory URL
        let url = osv.references.as_ref().and_then(|refs| {
            refs.iter()
                .find(|r| r.ref_type == "ADVISORY" || r.ref_type == "WEB")
                .map(|r| r.url.clone())
        });

        Vulnerability {
            id,
            severity,
            description,
            url,
        }
    }
}

impl Default for OsvClient {
    fn default() -> Self {
        Self::new().expect("Failed to create OsvClient")
    }
}

/// Parse CVSS score to severity level
fn parse_cvss_severity(score: &str) -> VulnerabilitySeverity {
    // Try to parse as CVSS score (float) first
    // CVSS v3 score ranges: 0-3.9 Low, 4-6.9 Medium, 7-8.9 High, 9-10 Critical
    if let Ok(score) = score.parse::<f64>() {
        return match score {
            s if s >= 9.0 => VulnerabilitySeverity::Critical,
            s if s >= 7.0 => VulnerabilitySeverity::High,
            s if s >= 4.0 => VulnerabilitySeverity::Medium,
            _ => VulnerabilitySeverity::Low,
        };
    }

    // Try to extract score from CVSS vector string (e.g., "CVSS:3.1/AV:N/AC:L/...")
    if score.starts_with("CVSS:") {
        // The score isn't directly in the vector, default to Medium
        return VulnerabilitySeverity::Medium;
    }

    // Default
    VulnerabilitySeverity::Medium
}

impl VulnerabilitySource for OsvClient {
    async fn query(&self, query: &VulnerabilityQuery) -> anyhow::Result<Vec<Vulnerability>> {
        let request = OsvQueryRequest {
            package: OsvPackage {
                name: query.package_name.clone(),
                ecosystem: query.ecosystem.as_osv_str().to_string(),
            },
            version: Some(query.version.clone()),
        };

        let url = format!("{}/query", self.base_url);
        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("OSV API error: {}", response.status());
        }

        let osv_response: OsvQueryResponse = response.json().await?;

        let vulns = osv_response
            .vulns
            .unwrap_or_default()
            .iter()
            .map(Self::convert_vulnerability)
            .collect();

        Ok(vulns)
    }

    async fn query_batch(
        &self,
        queries: &[VulnerabilityQuery],
    ) -> anyhow::Result<Vec<Vec<Vulnerability>>> {
        if queries.is_empty() {
            return Ok(vec![]);
        }

        let request = OsvBatchRequest {
            queries: queries
                .iter()
                .map(|q| OsvQueryRequest {
                    package: OsvPackage {
                        name: q.package_name.clone(),
                        ecosystem: q.ecosystem.as_osv_str().to_string(),
                    },
                    version: Some(q.version.clone()),
                })
                .collect(),
        };

        let url = format!("{}/querybatch", self.base_url);
        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("OSV API batch error: {}", response.status());
        }

        let batch_response: OsvBatchResponse = response.json().await?;

        let results = batch_response
            .results
            .iter()
            .map(|r| {
                r.vulns
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(Self::convert_vulnerability)
                    .collect()
            })
            .collect();

        Ok(results)
    }
}

// OSV API Request/Response structures

#[derive(Debug, Serialize)]
struct OsvQueryRequest {
    package: OsvPackage,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

#[derive(Debug, Serialize)]
struct OsvPackage {
    name: String,
    ecosystem: String,
}

#[derive(Debug, Serialize)]
struct OsvBatchRequest {
    queries: Vec<OsvQueryRequest>,
}

#[derive(Debug, Deserialize)]
struct OsvQueryResponse {
    vulns: Option<Vec<OsvVulnerability>>,
}

#[derive(Debug, Deserialize)]
struct OsvBatchResponse {
    results: Vec<OsvQueryResponse>,
}

#[derive(Debug, Deserialize)]
struct OsvVulnerability {
    id: String,
    summary: Option<String>,
    details: Option<String>,
    severity: Option<Vec<OsvSeverity>>,
    references: Option<Vec<OsvReference>>,
    #[allow(dead_code)]
    affected: Option<Vec<OsvAffected>>,
    aliases: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OsvSeverity {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    severity_type: String,
    score: String,
}

#[derive(Debug, Deserialize)]
struct OsvReference {
    #[serde(rename = "type")]
    ref_type: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct OsvAffected {
    #[allow(dead_code)]
    package: Option<OsvAffectedPackage>,
    #[allow(dead_code)]
    ranges: Option<Vec<OsvRange>>,
    #[allow(dead_code)]
    versions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OsvAffectedPackage {
    #[allow(dead_code)]
    ecosystem: String,
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct OsvRange {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    range_type: String,
    #[allow(dead_code)]
    events: Vec<OsvRangeEvent>,
}

#[derive(Debug, Deserialize)]
struct OsvRangeEvent {
    #[allow(dead_code)]
    introduced: Option<String>,
    #[allow(dead_code)]
    fixed: Option<String>,
    #[allow(dead_code)]
    last_affected: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vulnerabilities::Ecosystem;

    #[test]
    fn test_parse_cvss_severity() {
        assert_eq!(parse_cvss_severity("9.8"), VulnerabilitySeverity::Critical);
        assert_eq!(parse_cvss_severity("9.0"), VulnerabilitySeverity::Critical);
        assert_eq!(parse_cvss_severity("7.5"), VulnerabilitySeverity::High);
        assert_eq!(parse_cvss_severity("5.0"), VulnerabilitySeverity::Medium);
        assert_eq!(parse_cvss_severity("3.0"), VulnerabilitySeverity::Low);
        assert_eq!(parse_cvss_severity("0.0"), VulnerabilitySeverity::Low);
        assert_eq!(
            parse_cvss_severity("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            VulnerabilitySeverity::Medium
        );
    }

    #[test]
    fn test_ecosystem_osv_str() {
        assert_eq!(Ecosystem::CratesIo.as_osv_str(), "crates.io");
        assert_eq!(Ecosystem::Npm.as_osv_str(), "npm");
        assert_eq!(Ecosystem::PyPI.as_osv_str(), "PyPI");
        assert_eq!(Ecosystem::Go.as_osv_str(), "Go");
        assert_eq!(Ecosystem::Packagist.as_osv_str(), "Packagist");
        assert_eq!(Ecosystem::Pub.as_osv_str(), "Pub");
        assert_eq!(Ecosystem::NuGet.as_osv_str(), "NuGet");
    }

    #[test]
    fn test_convert_vulnerability() {
        let osv = OsvVulnerability {
            id: "GHSA-xxxx-xxxx-xxxx".to_string(),
            summary: Some("Test vulnerability".to_string()),
            details: None,
            severity: Some(vec![OsvSeverity {
                severity_type: "CVSS_V3".to_string(),
                score: "7.5".to_string(),
            }]),
            references: Some(vec![OsvReference {
                ref_type: "ADVISORY".to_string(),
                url: "https://example.com/advisory".to_string(),
            }]),
            affected: None,
            aliases: Some(vec!["CVE-2021-12345".to_string()]),
        };

        let vuln = OsvClient::convert_vulnerability(&osv);

        assert_eq!(vuln.id, "CVE-2021-12345");
        assert_eq!(vuln.severity, VulnerabilitySeverity::High);
        assert_eq!(vuln.description, "Test vulnerability");
        assert_eq!(vuln.url, Some("https://example.com/advisory".to_string()));
    }
}
