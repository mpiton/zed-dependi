//! Vulnerability report generation
//!
//! This module handles the generation of vulnerability reports
//! in various formats (JSON, Markdown).

use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VulnerabilitySummary {
    pub total: u32,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnerabilityReportEntry {
    pub package: String,
    pub version: String,
    pub id: String,
    pub severity: String,
    pub description: String,
    pub url: Option<String>,
}

pub fn generate_markdown_report(
    uri: &Url,
    summary: &VulnerabilitySummary,
    vulnerabilities: &[VulnerabilityReportEntry],
) -> String {
    let mut lines = vec![
        "# Vulnerability Report".to_string(),
        String::new(),
        format!("**File**: {}", uri.path()),
        format!("**Date**: {}", chrono::Local::now().format("%Y-%m-%d")),
        String::new(),
        "## Summary".to_string(),
        "| Severity | Count |".to_string(),
        "|----------|-------|".to_string(),
        format!("| ⚠ Critical | {} |", summary.critical),
        format!("| ▲ High | {} |", summary.high),
        format!("| ● Medium | {} |", summary.medium),
        format!("| ○ Low | {} |", summary.low),
        format!("| **Total** | **{}** |", summary.total),
        String::new(),
    ];

    if !vulnerabilities.is_empty() {
        lines.push("## Vulnerabilities".to_string());
        lines.push(String::new());

        let mut current_package = String::new();
        for vuln in vulnerabilities {
            if vuln.package != current_package {
                current_package = vuln.package.clone();
                lines.push(format!("### {}@{}", vuln.package, vuln.version));
                lines.push(String::new());
            }

            let severity_icon = match vuln.severity.as_str() {
                "critical" => "⚠",
                "high" => "▲",
                "medium" => "●",
                _ => "○",
            };

            if let Some(url) = &vuln.url {
                lines.push(format!(
                    "- **[{}]({})** ({} {}): {}",
                    vuln.id,
                    url,
                    severity_icon,
                    vuln.severity.to_uppercase(),
                    vuln.description
                ));
            } else {
                lines.push(format!(
                    "- **{}** ({} {}): {}",
                    vuln.id,
                    severity_icon,
                    vuln.severity.to_uppercase(),
                    vuln.description
                ));
            }
        }
    } else {
        lines.push("## No vulnerabilities found".to_string());
        lines.push(String::new());
        lines.push("✅ All dependencies are free of known security vulnerabilities.".to_string());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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
