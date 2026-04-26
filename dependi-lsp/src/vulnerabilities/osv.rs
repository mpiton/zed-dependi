//! OSV.dev API client for vulnerability data
//!
//! OSV (Open Source Vulnerabilities) provides a unified API for querying
//! vulnerability data across multiple ecosystems.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::VulnerabilityQuery;
use crate::registries::{Vulnerability, VulnerabilitySeverity};

const OSV_API_BASE: &str = "https://api.osv.dev/v1";
const RUSTSEC_ADVISORY_LOOKUP_CONCURRENCY: usize = 5;

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
    // Wired into `check_rustsec_unmaintained` in Task 22; until then the field
    // is only consumed via the test-only `advisory_cache()` accessor, so the
    // lib-only build legitimately sees no read.
    #[allow(dead_code)]
    advisory_cache: Arc<dyn crate::cache::advisory::AdvisoryWriteCache>,
}

impl OsvClient {
    pub fn new() -> anyhow::Result<Self> {
        Self::new_with_cache(Arc::new(crate::cache::advisory::NullAdvisoryCache))
    }

    pub fn new_with_cache(
        advisory_cache: Arc<dyn crate::cache::advisory::AdvisoryWriteCache>,
    ) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("dependi-lsp (https://github.com/mathieu/zed-dependi)")
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            client: Arc::new(client),
            base_url: OSV_API_BASE.to_string(),
            advisory_cache,
        })
    }

    /// Constructor used in tests to point the client at a mock server (no cache).
    pub fn with_endpoint(endpoint: String) -> Self {
        Self::with_endpoint_and_cache(
            endpoint,
            Arc::new(crate::cache::advisory::NullAdvisoryCache),
        )
    }

    /// Test/runtime constructor: explicit endpoint and cache.
    pub fn with_endpoint_and_cache(
        endpoint: String,
        advisory_cache: Arc<dyn crate::cache::advisory::AdvisoryWriteCache>,
    ) -> Self {
        Self {
            base_url: endpoint,
            client: Arc::new(
                Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()
                    .expect("reqwest client should build"),
            ),
            advisory_cache,
        }
    }

    /// Accessor used by integration tests to verify cache state.
    #[cfg(test)]
    pub fn advisory_cache(&self) -> &Arc<dyn crate::cache::advisory::AdvisoryWriteCache> {
        &self.advisory_cache
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

        let results: Vec<bool> = stream::iter(ids.iter().cloned())
            .map(|id| {
                let url = format!("{}/vulns/{id}", self.base_url);
                let client = Arc::clone(&self.client);

                async move {
                    let response = match client.get(&url).send().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!("Failed to fetch advisory {}: {}", id, e);
                            return false;
                        }
                    };

                    let details: Option<OsvVulnerabilityDetails> = match response.json().await {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("Failed to parse advisory {}: {}", id, e);
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
                            id,
                            details
                                .as_ref()
                                .and_then(|v| v.summary.as_ref())
                                .unwrap_or(&String::new())
                        );
                    }

                    is_unmaintained
                }
            })
            .buffer_unordered(RUSTSEC_ADVISORY_LOOKUP_CONCURRENCY)
            .collect()
            .await;

        results.into_iter().any(std::convert::identity)
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::vulnerabilities::Ecosystem;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_counting_osv_server(
        active_requests: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("counting test server should bind");
        let addr = listener
            .local_addr()
            .expect("counting test server should have local address");

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _peer)) = listener.accept().await else {
                    break;
                };
                let active_requests = Arc::clone(&active_requests);
                let max_seen = Arc::clone(&max_seen);

                tokio::spawn(async move {
                    let current = active_requests.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(current, Ordering::SeqCst);

                    let mut buffer = [0_u8; 2048];
                    let _ = socket.read(&mut buffer).await;

                    tokio::time::sleep(Duration::from_millis(50)).await;

                    let body =
                        r#"{"id":"RUSTSEC-2099-0001","summary":"ordinary advisory","affected":[]}"#;
                    let body_len = body.len();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {body_len}\r\nconnection: close\r\n\r\n{body}"
                    );
                    let _ = socket.write_all(response.as_bytes()).await;

                    active_requests.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        format!("http://{addr}")
    }

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

    #[tokio::test]
    async fn test_osv_client_with_endpoint_uses_custom_url() {
        let client = OsvClient::with_endpoint("http://127.0.0.1:1".to_string());
        // Port 1 is not listening; a real query must error, proving the client
        // actually attempted to reach the custom URL (and not fallen back to
        // api.osv.dev).
        let query = VulnerabilityQuery {
            ecosystem: Ecosystem::CratesIo,
            package_name: "serde".to_string(),
            version: "1.0.0".to_string(),
        };
        let result = client.query_batch(&[query]).await;
        assert!(
            result.is_err(),
            "query to unreachable endpoint should error, got {result:?}"
        );
    }

    use crate::cache::advisory::{AdvisoryReadCache, AdvisoryWriteCache, NullAdvisoryCache};

    #[tokio::test]
    async fn with_endpoint_accepts_advisory_cache() {
        let cache: Arc<dyn AdvisoryWriteCache> = Arc::new(NullAdvisoryCache);
        let client =
            OsvClient::with_endpoint_and_cache("http://example.invalid".to_string(), cache);
        // Smoke-test: cache is reachable through the public accessor.
        assert!(
            client
                .advisory_cache()
                .get("RUSTSEC-2020-0036")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_rustsec_advisory_lookup_limits_concurrency() {
        const EXPECTED_LIMIT: usize = RUSTSEC_ADVISORY_LOOKUP_CONCURRENCY;

        let active_requests = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let endpoint =
            spawn_counting_osv_server(Arc::clone(&active_requests), Arc::clone(&max_seen)).await;
        let client = OsvClient::with_endpoint(endpoint);
        let ids: Vec<String> = (0..(EXPECTED_LIMIT * 2 + 3))
            .map(|index| format!("RUSTSEC-2099-{index:04}"))
            .collect();

        let deprecated = client.check_rustsec_unmaintained(&ids).await;

        assert!(
            !deprecated,
            "test advisories should not be marked as unmaintained"
        );
        assert!(
            max_seen.load(Ordering::SeqCst) <= EXPECTED_LIMIT,
            "expected at most {EXPECTED_LIMIT} concurrent advisory lookups, saw {}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn test_normalize_version_strips_operators_for_osv() {
        use super::super::normalize_version_for_osv;

        // These are real-world version strings from Python pyproject.toml
        assert_eq!(normalize_version_for_osv(">=1.23.0"), "1.23.0");
        assert_eq!(normalize_version_for_osv("==2.0.0"), "2.0.0");
        assert_eq!(normalize_version_for_osv("~=4.0"), "4.0");
        // Cargo/npm style
        assert_eq!(normalize_version_for_osv("^1.0.27"), "1.0.27");
    }

    #[tokio::test]
    async fn test_version_normalization_prevents_false_positives() {
        use super::super::normalize_version_for_osv;

        // urllib3 1.26.0 should NOT have GHSA-m5vv-6r4h-3vj9 (only affects <1.16.1)
        let raw_version = ">=1.26.0";
        let normalized = normalize_version_for_osv(raw_version);
        assert_eq!(normalized, "1.26.0");

        let client = OsvClient::new().unwrap();
        let query = VulnerabilityQuery {
            ecosystem: crate::vulnerabilities::Ecosystem::PyPI,
            package_name: "urllib3".to_string(),
            version: normalized,
        };

        let results = client.query_batch(&[query]).await.unwrap();
        assert!(!results.is_empty());

        let result = &results[0];
        // Verify the specific false-positive vulnerability is NOT present
        let has_false_positive = result.vulnerabilities.iter().any(|v| {
            v.id.contains("GHSA-m5vv-6r4h-3vj9") || v.description.contains("GHSA-m5vv-6r4h-3vj9")
        });
        assert!(
            !has_false_positive,
            "urllib3 1.26.0 should NOT have GHSA-m5vv-6r4h-3vj9 (only affects <1.16.1)"
        );
    }
}
