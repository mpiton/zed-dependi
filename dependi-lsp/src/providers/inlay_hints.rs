//! Inlay hints provider for dependency versions

use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position};

use crate::parsers::Dependency;
use crate::registries::{VersionInfo, VulnerabilitySeverity};

/// Result of comparing a dependency version with the latest available
#[derive(Debug, Clone)]
pub enum VersionStatus {
    /// Version is up to date
    UpToDate,
    /// Update available to the given version
    UpdateAvailable(String),
    /// Could not determine version status
    Unknown,
}

/// Generate an inlay hint for a dependency
pub fn create_inlay_hint(dep: &Dependency, version_info: Option<&VersionInfo>) -> InlayHint {
    let status = match version_info {
        Some(info) => compare_versions(&dep.version, info),
        None => VersionStatus::Unknown,
    };

    // Check for vulnerabilities
    let vuln_count = version_info
        .map(|info| info.vulnerabilities.len())
        .unwrap_or(0);

    let (label, tooltip) = create_hint_label_and_tooltip(&status, vuln_count, dep, version_info);

    InlayHint {
        position: Position {
            line: dep.line,
            character: dep.version_end + 1,
        },
        label: InlayHintLabel::String(format!(" {}", label)),
        kind: Some(InlayHintKind::PARAMETER),
        text_edits: None,
        tooltip: tooltip.map(tower_lsp::lsp_types::InlayHintTooltip::String),
        padding_left: Some(true),
        padding_right: None,
        data: None,
    }
}

/// Create label and tooltip based on version status and vulnerabilities
fn create_hint_label_and_tooltip(
    status: &VersionStatus,
    vuln_count: usize,
    dep: &Dependency,
    version_info: Option<&VersionInfo>,
) -> (String, Option<String>) {
    // Handle vulnerabilities first (they take priority)
    if vuln_count > 0 {
        let vuln_label = format!("⚠ {}", vuln_count);
        let vuln_tooltip = format_vulnerability_tooltip(version_info.unwrap());

        // Combine with update info if available
        return match status {
            VersionStatus::UpdateAvailable(latest) => {
                let label = format!("{} ⬆ {}", vuln_label, latest);
                let tooltip = format!(
                    "{}\n\n---\n**Update available:** {} → {}",
                    vuln_tooltip, dep.version, latest
                );
                (label, Some(tooltip))
            }
            _ => (vuln_label, Some(vuln_tooltip)),
        };
    }

    // No vulnerabilities - show version status
    match status {
        VersionStatus::UpToDate => ("✓".to_string(), Some("Up to date".to_string())),
        VersionStatus::UpdateAvailable(latest) => {
            let label = format!("⬆ {}", latest);
            let tooltip = format!("Update available: {} → {}", dep.version, latest);
            (label, Some(tooltip))
        }
        VersionStatus::Unknown => (
            "?".to_string(),
            Some("Could not fetch version info".to_string()),
        ),
    }
}

/// Format vulnerability details for tooltip
fn format_vulnerability_tooltip(info: &VersionInfo) -> String {
    let mut lines = vec![format!(
        "**⚠ {} Security Vulnerabilities Found**\n",
        info.vulnerabilities.len()
    )];

    for (i, vuln) in info.vulnerabilities.iter().take(5).enumerate() {
        let severity_icon = match vuln.severity {
            VulnerabilitySeverity::Critical => "🔴 CRITICAL",
            VulnerabilitySeverity::High => "🟠 HIGH",
            VulnerabilitySeverity::Medium => "🟡 MEDIUM",
            VulnerabilitySeverity::Low => "🟢 LOW",
        };

        lines.push(format!(
            "{}. **{}** ({})\n   {}",
            i + 1,
            vuln.id,
            severity_icon,
            truncate_string(&vuln.description, 100)
        ));

        if let Some(url) = &vuln.url {
            lines.push(format!("   [View Advisory]({})", url));
        }
    }

    if info.vulnerabilities.len() > 5 {
        lines.push(format!(
            "\n... and {} more vulnerabilities",
            info.vulnerabilities.len() - 5
        ));
    }

    lines.join("\n")
}

/// Truncate a string to max length with ellipsis
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Compare a dependency version with the latest available
pub fn compare_versions(current: &str, info: &VersionInfo) -> VersionStatus {
    let Some(latest) = &info.latest else {
        return VersionStatus::Unknown;
    };

    // Normalize versions for comparison
    let current_normalized = normalize_version(current);
    let latest_normalized = normalize_version(latest);

    // Parse as semver for proper comparison
    match (
        semver::Version::parse(&current_normalized),
        semver::Version::parse(&latest_normalized),
    ) {
        (Ok(current_ver), Ok(latest_ver)) => {
            if current_ver >= latest_ver {
                VersionStatus::UpToDate
            } else {
                VersionStatus::UpdateAvailable(latest.clone())
            }
        }
        _ => {
            // Fallback to string comparison if semver parsing fails
            if current_normalized == latest_normalized {
                VersionStatus::UpToDate
            } else {
                VersionStatus::UpdateAvailable(latest.clone())
            }
        }
    }
}

/// Normalize a version string for comparison
/// Handles version specifiers like ^, ~, >=, etc.
fn normalize_version(version: &str) -> String {
    let version = version.trim();

    // Remove common prefixes
    let version = version
        .strip_prefix('^')
        .or_else(|| version.strip_prefix('~'))
        .or_else(|| version.strip_prefix(">="))
        .or_else(|| version.strip_prefix("<="))
        .or_else(|| version.strip_prefix('>'))
        .or_else(|| version.strip_prefix('<'))
        .or_else(|| version.strip_prefix('='))
        .unwrap_or(version);

    // Handle version ranges like ">=1.0, <2.0" - take the first part
    let version = version.split(',').next().unwrap_or(version).trim();

    // Ensure we have at least major.minor.patch
    let parts: Vec<&str> = version.split('.').collect();
    match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_version_info(latest: &str) -> VersionInfo {
        VersionInfo {
            latest: Some(latest.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_compare_versions_up_to_date() {
        let info = make_version_info("1.0.0");
        assert!(matches!(
            compare_versions("1.0.0", &info),
            VersionStatus::UpToDate
        ));
    }

    #[test]
    fn test_compare_versions_update_available() {
        let info = make_version_info("2.0.0");
        match compare_versions("1.0.0", &info) {
            VersionStatus::UpdateAvailable(v) => assert_eq!(v, "2.0.0"),
            _ => panic!("Expected UpdateAvailable"),
        }
    }

    #[test]
    fn test_compare_versions_with_caret() {
        let info = make_version_info("1.5.0");
        match compare_versions("^1.0", &info) {
            VersionStatus::UpdateAvailable(v) => assert_eq!(v, "1.5.0"),
            _ => panic!("Expected UpdateAvailable"),
        }
    }

    #[test]
    fn test_compare_versions_with_tilde() {
        let info = make_version_info("1.0.5");
        match compare_versions("~1.0.0", &info) {
            VersionStatus::UpdateAvailable(v) => assert_eq!(v, "1.0.5"),
            _ => panic!("Expected UpdateAvailable"),
        }
    }

    #[test]
    fn test_normalize_version() {
        assert_eq!(normalize_version("1.0.0"), "1.0.0");
        assert_eq!(normalize_version("^1.0"), "1.0.0");
        assert_eq!(normalize_version("~1.0.0"), "1.0.0");
        assert_eq!(normalize_version(">=1.0, <2.0"), "1.0.0");
        assert_eq!(normalize_version("1"), "1.0.0");
        assert_eq!(normalize_version("1.2"), "1.2.0");
    }

    #[test]
    fn test_create_inlay_hint_up_to_date() {
        let dep = Dependency {
            name: "serde".to_string(),
            version: "1.0.0".to_string(),
            line: 5,
            name_start: 0,
            name_end: 5,
            version_start: 9,
            version_end: 16,
            dev: false,
            optional: false,
        };
        let info = make_version_info("1.0.0");
        let hint = create_inlay_hint(&dep, Some(&info));

        assert_eq!(hint.position.line, 5);
        match hint.label {
            InlayHintLabel::String(s) => assert!(s.contains("✓")),
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_create_inlay_hint_update_available() {
        let dep = Dependency {
            name: "serde".to_string(),
            version: "1.0.0".to_string(),
            line: 5,
            name_start: 0,
            name_end: 5,
            version_start: 9,
            version_end: 16,
            dev: false,
            optional: false,
        };
        let info = make_version_info("2.0.0");
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("⬆"));
                assert!(s.contains("2.0.0"));
            }
            _ => panic!("Expected string label"),
        }
    }
}
