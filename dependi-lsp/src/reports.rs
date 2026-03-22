//! Vulnerability report generation
//!
//! This module handles the generation of vulnerability reports
//! in various formats (JSON, Markdown).

use core::fmt;

use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

/// Summary of vulnerabilities grouped by severity level.
///
/// Used to provide an overview of the vulnerability scan results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VulnerabilitySummary {
    /// Total number of vulnerabilities found.
    pub total: u32,
    /// Number of critical severity vulnerabilities.
    pub critical: u32,
    /// Number of high severity vulnerabilities.
    pub high: u32,
    /// Number of medium severity vulnerabilities.
    pub medium: u32,
    /// Number of low severity vulnerabilities.
    pub low: u32,
}

/// A single vulnerability entry in a report.
///
/// Contains all relevant information about a vulnerability affecting a package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnerabilityReportEntry {
    /// Name of the affected package.
    pub package: String,
    /// Version of the affected package.
    pub version: String,
    /// Vulnerability identifier (e.g., CVE-2021-1234, GHSA-xxxx).
    pub id: String,
    /// Severity level (critical, high, medium, low).
    pub severity: String,
    /// Human-readable description of the vulnerability.
    pub description: String,
    /// URL for more information about the vulnerability.
    pub url: Option<String>,
}

/// Returns an <code>[fmt::Display] + [fmt::Debug]</code> implementation
/// which produces a Markdown-formatted vulnerability report.
///
/// Creates a human-readable report with a summary table and detailed
/// vulnerability entries grouped by package.
#[must_use = "returns a type implementing Display and Debug, which does not have any effects unless they are used"]
pub fn fmt_markdown_report(
    uri: &Url,
    summary: &VulnerabilitySummary,
    vulnerabilities: &[VulnerabilityReportEntry],
) -> impl fmt::Display + fmt::Debug {
    fmt::from_fn(move |f| {
        writeln!(
            f,
            "# Vulnerability Report\n\
             \n\
             **File**: {}",
            uri.path()
        )?;
        writeln!(f, "**Date**: {}\n", chrono::Local::now().format("%Y-%m-%d"))?;
        writeln!(
            f,
            "## Summary\n\
             \n\
             | Severity | Count |\n\
             |----------|-------|\n\
             | ⚠ Critical | {c} |\n\
             | ▲ High | {h} |\n\
             | ● Medium | {m} |\n\
             | ○ Low | {l} |\n\
             | **Total** | **{t}** |\n",
            c = summary.critical,
            h = summary.high,
            m = summary.medium,
            l = summary.low,
            t = summary.total,
        )?;

        if !vulnerabilities.is_empty() {
            writeln!(f, "## Vulnerabilities\n")?;

            let mut current_package = String::new();
            let mut current_version = String::new();
            for VulnerabilityReportEntry {
                package,
                version,
                id,
                severity,
                description,
                url,
            } in vulnerabilities
            {
                if *package != current_package || *version != current_version {
                    current_package = package.clone();
                    current_version = version.clone();
                    writeln!(f, "### {package}@{version}\n")?;
                }

                let severity_icon = match severity.as_str() {
                    "critical" => "⚠",
                    "high" => "▲",
                    "medium" => "●",
                    _ => "○",
                };

                let severity = severity.to_uppercase();
                if let Some(url) = url.as_deref() {
                    writeln!(
                        f,
                        "- **[{id}]({url})** ({severity_icon} {severity}): {description}",
                    )?;
                } else {
                    writeln!(f, "- **{id}** ({severity_icon} {severity}): {description}")?;
                }
            }
        } else {
            writeln!(
                f,
                "## No vulnerabilities found\n\
                 ✅ All dependencies are free of known security vulnerabilities."
            )?;
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_markdown_report(
        uri: &Url,
        summary: &VulnerabilitySummary,
        vulnerabilities: &[VulnerabilityReportEntry],
    ) -> String {
        fmt_markdown_report(uri, summary, vulnerabilities).to_string()
    }

    #[test]
    fn test_generate_markdown_report_with_vulnerabilities() {
        let uri = Url::parse("file:///project/Cargo.toml").unwrap();
        let summary = VulnerabilitySummary {
            total: 2,
            critical: 1,
            high: 1,
            medium: 0,
            low: 0,
        };
        let vulnerabilities = vec![
            VulnerabilityReportEntry {
                package: "serde".to_string(),
                version: "1.0.0".to_string(),
                id: "CVE-2021-1234".to_string(),
                severity: "critical".to_string(),
                description: "Critical vulnerability".to_string(),
                url: Some("https://example.com/cve".to_string()),
            },
            VulnerabilityReportEntry {
                package: "tokio".to_string(),
                version: "1.0.0".to_string(),
                id: "CVE-2021-5678".to_string(),
                severity: "high".to_string(),
                description: "High vulnerability".to_string(),
                url: None,
            },
        ];

        let report = generate_markdown_report(&uri, &summary, &vulnerabilities);

        assert!(report.contains("# Vulnerability Report"));
        assert!(report.contains("**File**: /project/Cargo.toml"));
        assert!(report.contains("| ⚠ Critical | 1 |"));
        assert!(report.contains("| ▲ High | 1 |"));
        assert!(report.contains("### serde@1.0.0"));
        assert!(report.contains("### tokio@1.0.0"));
        assert!(report.contains("CVE-2021-1234"));
        assert!(report.contains("CVE-2021-5678"));
    }

    #[test]
    fn test_generate_markdown_report_same_package_different_versions() {
        let uri = Url::parse("file:///project/Cargo.toml").unwrap();
        let summary = VulnerabilitySummary {
            total: 2,
            critical: 1,
            high: 1,
            medium: 0,
            low: 0,
        };
        let vulnerabilities = vec![
            VulnerabilityReportEntry {
                package: "serde".to_string(),
                version: "1.0.0".to_string(),
                id: "CVE-2021-1111".to_string(),
                severity: "critical".to_string(),
                description: "Old version vulnerability".to_string(),
                url: None,
            },
            VulnerabilityReportEntry {
                package: "serde".to_string(),
                version: "2.0.0".to_string(),
                id: "CVE-2021-2222".to_string(),
                severity: "high".to_string(),
                description: "New version vulnerability".to_string(),
                url: None,
            },
        ];

        let report = generate_markdown_report(&uri, &summary, &vulnerabilities);

        assert!(report.contains("### serde@1.0.0"));
        assert!(report.contains("### serde@2.0.0"));
        assert!(report.contains("CVE-2021-1111"));
        assert!(report.contains("CVE-2021-2222"));
    }

    #[test]
    fn test_generate_markdown_report_no_vulnerabilities() {
        let uri = Url::parse("file:///project/Cargo.toml").unwrap();
        let summary = VulnerabilitySummary::default();
        let vulnerabilities = vec![];

        let report = generate_markdown_report(&uri, &summary, &vulnerabilities);

        assert!(report.contains("# Vulnerability Report"));
        assert!(report.contains("## No vulnerabilities found"));
        assert!(report.contains("✅ All dependencies are free of known security vulnerabilities."));
    }
}
