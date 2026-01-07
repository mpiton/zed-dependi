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

    // Check for vulnerabilities and deprecation
    let vuln_count = version_info
        .map(|info| info.vulnerabilities.len())
        .unwrap_or(0);

    let is_deprecated = version_info.map(|info| info.deprecated).unwrap_or(false);

    if is_deprecated {
        tracing::debug!("Package {} {} is deprecated", dep.name, dep.version);
    }

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
    // Handle yanked versions (highest priority - critical issue)
    if let Some(info) = version_info
        && info.is_version_yanked(&dep.version)
    {
        let yanked_label = "🚫 Yanked".to_string();
        let yanked_tooltip = format_yanked_tooltip(dep, info);

        return match status {
            VersionStatus::UpdateAvailable(latest) => {
                let label = format!("{} -> {}", yanked_label, latest);
                let tooltip = format!(
                    "{}\n\n---\n**Update available:** {} -> {}",
                    yanked_tooltip, dep.version, latest
                );
                (label, Some(tooltip))
            }
            _ => (yanked_label, Some(yanked_tooltip)),
        };
    }

    // Handle deprecation (second highest priority)
    if let Some(info) = version_info
        && info.deprecated
    {
        let dep_label = "⚠ Deprecated".to_string();
        let dep_tooltip = format_deprecation_tooltip(dep, info);

        // Combine with update info if available
        return match status {
            VersionStatus::UpdateAvailable(latest) => {
                let label = format!("{} -> {}", dep_label, latest);
                let tooltip = format!(
                    "{}\n\n---\n**Update available:** {} -> {}",
                    dep_tooltip, dep.version, latest
                );
                (label, Some(tooltip))
            }
            _ => (dep_label, Some(dep_tooltip)),
        };
    }

    // Handle vulnerabilities
    if vuln_count > 0 {
        let vuln_label = format!("⚠ {}", vuln_count);
        let vuln_tooltip = format_vulnerability_tooltip(version_info.unwrap());

        // Combine with update info if available
        return match status {
            VersionStatus::UpdateAvailable(latest) => {
                let label = format!("{} ⬆ {}", vuln_label, latest);
                let tooltip = format!(
                    "{}\n\n---\n**Update available:** {} -> {}",
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
            let tooltip = format!("Update available: {} -> {}", dep.version, latest);
            (label, Some(tooltip))
        }
        VersionStatus::Unknown => {
            if is_local_dependency(&dep.version) {
                let tooltip = format!(
                    "**Local Dependency**\n\n\
                    \"{}\" is a local/path dependency.\n\n\
                    Version info is not available for local packages.\n\n\
                    This is expected for dependencies using:\n\
                    • path = \"./...\"\n\
                    • git = \"https://...\"\n\
                    • git = \"git@...\"\n\
                    • github:owner/repo",
                    dep.name
                );
                ("📦 Local".to_string(), Some(tooltip))
            } else {
                let tooltip = format!(
                    "**Could not fetch version info**\n\n\
                    Possible causes:\n\
                    • Network error - check internet connection\n\
                    • Package not found - verify spelling\n\
                    • Rate limiting - wait and retry\n\
                    • Registry down - try again later\n\n\
                    **Troubleshooting:**\n\
                    1. Check your network connection\n\
                    2. Verify the package name \"{}\" is spelled correctly\n\
                    3. Search for the package on the registry\n\
                    4. If recently published, wait a few minutes for indexing",
                    dep.name
                );
                ("⚡".to_string(), Some(tooltip))
            }
        }
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

/// Format deprecation warning for tooltip
fn format_deprecation_tooltip(dep: &Dependency, info: &VersionInfo) -> String {
    let mut lines = vec![
        format!(
            "**⚠️ PACKAGE DEPRECATED**\n\nThe package \"{}\" is deprecated.",
            dep.name
        ),
        "".to_string(),
        "**Why was it deprecated?**".to_string(),
        "• Superseded by a better alternative".to_string(),
        "• Has unresolved critical bugs".to_string(),
        "• Known security vulnerabilities".to_string(),
        "• No longer maintained".to_string(),
        "• Has been renamed".to_string(),
        "".to_string(),
        "**What should you do?**".to_string(),
    ];

    if let Some(homepage) = &info.homepage {
        lines.push(format!("• Check the package homepage: {}", homepage));
    }

    if let Some(repo) = &info.repository {
        lines.push(format!(
            "• View the repository for more information: {}",
            repo
        ));
    }

    lines.push("• Search for an alternative package on the registry".to_string());

    if let Some(latest) = &info.latest {
        lines.push(format!(
            "• Consider the latest version: {} (may not be deprecated)",
            latest
        ));
    }

    lines.join("\n")
}

/// Format yanked version warning for tooltip
fn format_yanked_tooltip(dep: &Dependency, info: &VersionInfo) -> String {
    let mut lines = vec![
        format!(
            "**🚫 YANKED VERSION**\n\nThe version \"{}\" of \"{}\" has been yanked from crates.io.",
            dep.version, dep.name
        ),
        "".to_string(),
        "**Why was it yanked?**".to_string(),
        "A yanked version typically has:".to_string(),
        "• Critical bugs that break functionality".to_string(),
        "• Security vulnerabilities".to_string(),
        "• Published by mistake".to_string(),
        "• Corrupted or incomplete package".to_string(),
        "".to_string(),
        "**What should you do?**".to_string(),
    ];

    if let Some(latest) = &info.latest {
        lines.push(format!("• Update to the latest version: {}", latest));
    }

    if let Some(repo) = &info.repository {
        lines.push(format!("• Check the repository for more info: {}", repo));
    }

    lines.push(format!(
        "• View on crates.io: https://crates.io/crates/{}",
        dep.name
    ));

    lines.join("\n")
}

/// Check if a dependency version string indicates a local/path dependency
fn is_local_dependency(version: &str) -> bool {
    version.starts_with("./")
        || version.starts_with("../")
        || version.starts_with('/')
        || version.starts_with("file://")
        || version.starts_with("git+")
        || version.starts_with("git@")
        || version.starts_with("https://")
        || version.starts_with("http://")
        || version.contains("github:")
        || version.contains("gitlab:")
        || version.contains("bitbucket:")
        || version.starts_with("workspace:")
        || version.starts_with("link:")
        || version.starts_with("portal:")
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
    use crate::registries::Vulnerability;

    fn make_version_info(latest: &str) -> VersionInfo {
        VersionInfo {
            latest: Some(latest.to_string()),
            ..Default::default()
        }
    }

    fn make_test_dep(name: &str, version: &str) -> Dependency {
        Dependency {
            name: name.to_string(),
            version: version.to_string(),
            line: 5,
            name_start: 0,
            name_end: name.len() as u32,
            version_start: name.len() as u32 + 4,
            version_end: name.len() as u32 + 4 + version.len() as u32,
            dev: false,
            optional: false,
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
        let dep = make_test_dep("serde", "1.0.0");
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
        let dep = make_test_dep("serde", "1.0.0");
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

    #[test]
    fn test_deprecated_inlay_hint() {
        let dep = make_test_dep("old-dep", "1.0.0");
        let info = VersionInfo {
            deprecated: true,
            latest: Some("2.0.0".to_string()),
            description: Some("Superseded by new-package".to_string()),
            homepage: Some("https://example.com".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("Deprecated"));
                assert!(s.contains("⚠"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_deprecated_with_update() {
        let dep = make_test_dep("old-dep", "1.0.0");
        let info = VersionInfo {
            deprecated: true,
            latest: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("Deprecated"));
                assert!(s.contains("2.0.0"));
                assert!(s.contains("->"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_no_deprecated_warning() {
        let dep = make_test_dep("serde", "1.0.0");
        let info = VersionInfo {
            deprecated: false,
            latest: Some("1.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(!s.contains("Deprecated"));
                assert!(!s.contains("⚠"));
                assert!(s.contains("✓"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_deprecated_priority_over_vulnerabilities() {
        let dep = make_test_dep("dep", "1.0.0");
        let info = VersionInfo {
            deprecated: true,
            latest: None,
            vulnerabilities: vec![Vulnerability {
                id: "CVE-2024-1234".to_string(),
                severity: VulnerabilitySeverity::High,
                description: "Test vulnerability".to_string(),
                url: None,
            }],
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("Deprecated"));
                assert!(!s.contains("1"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_yanked_inlay_hint() {
        let dep = make_test_dep("serde", "1.0.0");
        let info = VersionInfo {
            yanked_versions: vec!["1.0.0".to_string()],
            latest: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("🚫"));
                assert!(s.contains("Yanked"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_yanked_with_update() {
        let dep = make_test_dep("serde", "1.0.0");
        let info = VersionInfo {
            yanked_versions: vec!["1.0.0".to_string()],
            latest: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("🚫"));
                assert!(s.contains("Yanked"));
                assert!(s.contains("2.0.0"));
                assert!(s.contains("->"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_no_yanked_warning() {
        let dep = make_test_dep("serde", "1.0.0");
        let info = VersionInfo {
            yanked_versions: vec!["0.9.0".to_string()],
            latest: Some("1.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(!s.contains("🚫"));
                assert!(!s.contains("Yanked"));
                assert!(s.contains("✓"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_yanked_priority_over_deprecated() {
        let dep = make_test_dep("dep", "1.0.0");
        let info = VersionInfo {
            yanked_versions: vec!["1.0.0".to_string()],
            deprecated: true,
            latest: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("🚫"));
                assert!(s.contains("Yanked"));
                assert!(!s.contains("Deprecated"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_yanked_priority_over_vulnerabilities() {
        let dep = make_test_dep("dep", "1.0.0");
        let info = VersionInfo {
            yanked_versions: vec!["1.0.0".to_string()],
            vulnerabilities: vec![Vulnerability {
                id: "CVE-2024-1234".to_string(),
                severity: VulnerabilitySeverity::High,
                description: "Test vulnerability".to_string(),
                url: None,
            }],
            latest: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let hint = create_inlay_hint(&dep, Some(&info));

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("🚫"));
                assert!(s.contains("Yanked"));
                assert!(!s.contains("⚠"));
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn test_is_local_dependency() {
        assert!(is_local_dependency("./my-lib"));
        assert!(is_local_dependency("../other-lib"));
        assert!(is_local_dependency("/absolute/path"));
        assert!(is_local_dependency("file:///some/path"));
        assert!(is_local_dependency("git+https://github.com/user/repo"));
        assert!(is_local_dependency("git@github.com:user/repo.git"));
        assert!(is_local_dependency("https://github.com/user/repo"));
        assert!(is_local_dependency("github:user/repo"));
        assert!(is_local_dependency("gitlab:user/repo"));
        assert!(is_local_dependency("bitbucket:user/repo"));
        assert!(is_local_dependency("workspace:*"));
        assert!(is_local_dependency("link:./my-lib"));
        assert!(is_local_dependency("portal:./my-lib"));

        assert!(!is_local_dependency("1.0.0"));
        assert!(!is_local_dependency("^1.0"));
        assert!(!is_local_dependency("~1.0.0"));
        assert!(!is_local_dependency(">=1.0, <2.0"));
        assert!(!is_local_dependency("*"));
    }

    #[test]
    fn test_unknown_status_local_dependency() {
        let dep = make_test_dep("my-local-lib", "./my-local-lib");
        let hint = create_inlay_hint(&dep, None);

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("📦"));
                assert!(s.contains("Local"));
            }
            _ => panic!("Expected string label"),
        }

        if let Some(tower_lsp::lsp_types::InlayHintTooltip::String(tooltip)) = hint.tooltip {
            assert!(tooltip.contains("Local Dependency"));
            assert!(tooltip.contains("my-local-lib"));
        } else {
            panic!("Expected string tooltip");
        }
    }

    #[test]
    fn test_unknown_status_network_error() {
        let dep = make_test_dep("unknown-package", "1.0.0");
        let hint = create_inlay_hint(&dep, None);

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("⚡"));
                assert!(!s.contains("Local"));
            }
            _ => panic!("Expected string label"),
        }

        if let Some(tower_lsp::lsp_types::InlayHintTooltip::String(tooltip)) = hint.tooltip {
            assert!(tooltip.contains("Could not fetch version info"));
            assert!(tooltip.contains("unknown-package"));
            assert!(tooltip.contains("Troubleshooting"));
        } else {
            panic!("Expected string tooltip");
        }
    }

    #[test]
    fn test_unknown_status_git_dependency() {
        let dep = make_test_dep("git-dep", "git@github.com:user/repo.git");
        let hint = create_inlay_hint(&dep, None);

        match hint.label {
            InlayHintLabel::String(s) => {
                assert!(s.contains("📦"));
                assert!(s.contains("Local"));
            }
            _ => panic!("Expected string label"),
        }
    }
}
