//! OSV.dev API client for vulnerability data
//!
//! OSV (Open Source Vulnerabilities) provides a unified API for querying
//! vulnerability data across multiple ecosystems.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::VulnerabilityQuery;
use crate::registries::{Vulnerability, VulnerabilitySeverity};

const OSV_API_BASE: &str = "https://api.osv.dev/v1";

/// Result of a vulnerability query
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub vulnerabilities: Vec<Vulnerability>,
    pub deprecated: bool,
}

/// OSV.dev API client
pub struct OsvClient {
    client: Arc<Client>,
    base_url: String,
}

impl OsvClient {
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

    fn convert_vulnerability(osv: &OsvVulnerability) -> Vulnerability {
        let id = osv
            .aliases
            .as_ref()
            .and_then(|a| a.iter().find(|id| id.starts_with("CVE-")))
            .cloned()
            .unwrap_or_else(|| osv.id.clone());

        let severity = osv
            .severity
            .as_ref()
            .and_then(|s| s.first())
            .map(|s| parse_cvss_severity(&s.score))
            .unwrap_or(VulnerabilitySeverity::Medium);

        let description = osv
            .summary
            .clone()
            .or_else(|| osv.details.clone())
            .unwrap_or_else(|| format!("Vulnerability {}", osv.id));

        let url = osv.references.as_ref().and_then(|refs| {
            refs.iter()
                .find(|r| r._ref_type == "ADVISORY" || r._ref_type == "WEB")
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

fn parse_cvss_severity(score: &str) -> VulnerabilitySeverity {
    if let Ok(score) = score.parse::<f64>() {
        return match score {
            s if s >= 9.0 => VulnerabilitySeverity::Critical,
            s if s >= 7.0 => VulnerabilitySeverity::High,
            s if s >= 4.0 => VulnerabilitySeverity::Medium,
            _ => VulnerabilitySeverity::Low,
        };
    }

    if score.starts_with("CVSS:") {
        return VulnerabilitySeverity::Medium;
    }

    VulnerabilitySeverity::Medium
}

impl OsvClient {
    pub async fn query_batch(
        &self,
        queries: &[VulnerabilityQuery],
    ) -> anyhow::Result<Vec<QueryResult>> {
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

        let mut results = Vec::new();

        for r in &batch_response.results {
            let vulnerabilities = r
                .vulns
                .as_ref()
                .unwrap_or(&vec![])
                .iter()
                .map(Self::convert_vulnerability)
                .collect();

            let rustsec_ids: Vec<String> = r
                .vulns
                .as_ref()
                .unwrap_or(&vec![])
                .iter()
                .filter(|v| v.id.starts_with("RUSTSEC-"))
                .map(|v| v.id.clone())
                .collect();

            let has_unmaintained = self.check_rustsec_unmaintained(&rustsec_ids).await;

            results.push(QueryResult {
                vulnerabilities,
                deprecated: has_unmaintained,
            });
        }

        Ok(results)
    }

    async fn check_rustsec_unmaintained(&self, ids: &[String]) -> bool {
        if ids.is_empty() {
            return false;
        }

        tracing::debug!(
            "Checking {} RUSTSEC advisories for unmaintained status",
            ids.len()
        );

        // Spawn all tasks in parallel
        let tasks: Vec<_> = ids
            .iter()
            .map(|id| {
                let url = format!("{}/vulns/{}", self.base_url, id);
                let client = Arc::clone(&self.client);
                let id_clone = id.clone();

                tokio::spawn(async move {
                    let response = match client.get(&url).send().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!("Failed to fetch advisory {}: {}", id_clone, e);
                            return false;
                        }
                    };

                    let details: Option<OsvVulnerabilityDetails> = match response.json().await {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("Failed to parse advisory {}: {}", id_clone, e);
                            return false;
                        }
                    };

                    let is_unmaintained = details.as_ref().is_some_and(|v| {
                        // Check summary for "maintained" or "deprecated" keywords
                        let summary_match = v.summary.as_ref().is_some_and(|s| {
                            let lower = s.to_lowercase();
                            lower.contains("maintained") || lower.contains("deprecated")
                        });

                        // Check database_specific.informational for "unmaintained"
                        let informational_match = v.affected.as_ref().is_some_and(|affected| {
                            affected.iter().any(|a| {
                                a.database_specific.as_ref().is_some_and(|db| {
                                    db.informational
                                        .as_ref()
                                        .is_some_and(|i| i == "unmaintained")
                                })
                            })
                        });

                        summary_match || informational_match
                    });

                    if is_unmaintained {
                        tracing::info!(
                            "Advisory {} indicates unmaintained package: {}",
                            id_clone,
                            details
                                .as_ref()
                                .and_then(|v| v.summary.as_ref())
                                .unwrap_or(&String::new())
                        );
                    }

                    is_unmaintained
                })
            })
            .collect();

        // Wait for ALL tasks to complete in parallel using join_all
        let results = futures::future::join_all(tasks).await;

        // Check if any task returned true (found unmaintained package)
        results.into_iter().any(|r| r.unwrap_or(false))
    }
}

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
    aliases: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    _type: String,
    score: String,
}

#[derive(Debug, Deserialize)]
struct OsvReference {
    #[serde(rename = "type")]
    _ref_type: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct OsvVulnerabilityDetails {
    summary: Option<String>,
    affected: Option<Vec<OsvAffected>>,
}

#[derive(Debug, Deserialize)]
struct OsvAffected {
    database_specific: Option<OsvDatabaseSpecific>,
}

#[derive(Debug, Deserialize)]
struct OsvDatabaseSpecific {
    informational: Option<String>,
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
                _type: "CVSS_V3".to_string(),
                score: "7.5".to_string(),
            }]),
            references: Some(vec![OsvReference {
                _ref_type: "ADVISORY".to_string(),
                url: "https://example.com/advisory".to_string(),
            }]),
            aliases: Some(vec!["CVE-2021-12345".to_string()]),
        };

        let vuln = OsvClient::convert_vulnerability(&osv);

        assert_eq!(vuln.id, "CVE-2021-12345");
        assert_eq!(vuln.severity, VulnerabilitySeverity::High);
        assert_eq!(vuln.description, "Test vulnerability");
        assert_eq!(vuln.url, Some("https://example.com/advisory".to_string()));
    }

    #[test]
    fn test_unmaintained_detection() {
        let rustsec_vuln = OsvVulnerability {
            id: "RUSTSEC-2025-0057".to_string(),
            summary: Some("fxhash - no longer maintained".to_string()),
            details: None,
            severity: None,
            references: None,
            aliases: None,
        };

        let is_unmaintained = rustsec_vuln.id.starts_with("RUSTSEC")
            && rustsec_vuln.summary.as_ref().is_some_and(|s| {
                s.to_lowercase().contains("maintained") || s.to_lowercase().contains("deprecated")
            });

        assert!(is_unmaintained);

        let normal_vuln = OsvVulnerability {
            id: "CVE-2024-1234".to_string(),
            summary: Some("Buffer overflow vulnerability".to_string()),
            details: None,
            severity: None,
            references: None,
            aliases: None,
        };

        let is_not_unmaintained = normal_vuln.id.starts_with("RUSTSEC")
            && normal_vuln.summary.as_ref().is_some_and(|s| {
                s.to_lowercase().contains("maintained") || s.to_lowercase().contains("deprecated")
            });

        assert!(!is_not_unmaintained);
    }

    #[tokio::test]
    async fn test_fxhash_deprecated_detection() {
        let client = OsvClient::new().unwrap();

        let query = VulnerabilityQuery {
            ecosystem: crate::vulnerabilities::Ecosystem::CratesIo,
            package_name: "fxhash".to_string(),
            version: "0.2.1".to_string(),
        };

        let results = client.query_batch(&[query]).await.unwrap();

        assert!(!results.is_empty());

        let result = &results[0];
        assert!(!result.vulnerabilities.is_empty());
        assert!(result.deprecated, "fxhash should be marked as deprecated");

        let vuln = &result.vulnerabilities[0];
        assert!(vuln.id.starts_with("RUSTSEC"));
    }
}
