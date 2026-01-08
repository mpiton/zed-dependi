//! Diagnostics provider for outdated dependencies and vulnerabilities

use tower_lsp::lsp_types::*;

use crate::cache::ReadCache;
use crate::parsers::Dependency;
use crate::providers::inlay_hints::{VersionStatus, compare_versions, is_local_dependency};
use crate::registries::{VersionInfo, Vulnerability, VulnerabilitySeverity};
use crate::utils::truncate_string;

/// Create diagnostics for a list of dependencies
///
/// The `min_severity` parameter filters vulnerabilities to only show those
/// at or above the specified severity level.
pub fn create_diagnostics(
    dependencies: &[Dependency],
    cache: &impl ReadCache,
    cache_key_fn: impl Fn(&str) -> String,
    min_severity: Option<VulnerabilitySeverity>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for dep in dependencies {
        // Show informational diagnostic for local/path dependencies
        if is_local_dependency(&dep.version) {
            diagnostics.push(create_local_dependency_diagnostic(dep));
            continue;
        }

        // Add outdated version diagnostic
        if let Some(diag) = create_outdated_diagnostic(dep, cache, &cache_key_fn) {
            diagnostics.push(diag);
        }

        let cache_key = cache_key_fn(&dep.name);
        if let Some(version_info) = cache.get(&cache_key) {
            // Add yanked version diagnostic (highest priority)
            if version_info.is_version_yanked(&dep.version) {
                tracing::debug!(
                    "Package {} {} is yanked, creating diagnostic",
                    dep.name,
                    dep.version
                );
                diagnostics.push(create_yanked_diagnostic(dep, &version_info));
            } else if version_info.deprecated {
                // Add deprecation diagnostic
                tracing::debug!(
                    "Package {} {} is deprecated, creating diagnostic",
                    dep.name,
                    dep.version
                );
                diagnostics.push(create_deprecation_diagnostic(dep, &version_info));
            } else {
                // Add vulnerability diagnostic (summary) only if not deprecated or yanked
                let filtered_vulns: Vec<_> = version_info
                    .vulnerabilities
                    .iter()
                    .filter(|vuln| {
                        min_severity
                            .as_ref()
                            .map(|min| meets_severity_threshold(&vuln.severity, min))
                            .unwrap_or(true)
                    })
                    .collect();

                if !filtered_vulns.is_empty() {
                    diagnostics.push(create_vulnerability_summary_diagnostic(
                        dep,
                        &filtered_vulns,
                    ));
                }
            }
        }
    }

    diagnostics
}

/// Check if a vulnerability severity meets the minimum threshold
fn meets_severity_threshold(severity: &VulnerabilitySeverity, min: &VulnerabilitySeverity) -> bool {
    severity.meets_threshold(min)
}

/// Create a diagnostic for an outdated dependency
fn create_outdated_diagnostic(
    dep: &Dependency,
    cache: &impl ReadCache,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<Diagnostic> {
    let cache_key = cache_key_fn(&dep.name);
    let version_info = cache.get(&cache_key)?;

    match compare_versions(&dep.version, &version_info) {
        VersionStatus::UpdateAvailable(new_version) => Some(Diagnostic {
            range: Range {
                start: Position {
                    line: dep.line,
                    character: dep.version_start,
                },
                end: Position {
                    line: dep.line,
                    character: dep.version_end,
                },
            },
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(NumberOrString::String("outdated".to_string())),
            source: Some("dependi".to_string()),
            message: format!("Update available: {} -> {}", dep.version, new_version),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        }),
        VersionStatus::UpToDate | VersionStatus::Unknown => None,
    }
}

/// Create a diagnostic for a local/path dependency
fn create_local_dependency_diagnostic(dep: &Dependency) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position {
                line: dep.line,
                character: dep.version_start,
            },
            end: Position {
                line: dep.line,
                character: dep.version_end,
            },
        },
        severity: Some(DiagnosticSeverity::HINT),
        code: Some(NumberOrString::String("local".to_string())),
        source: Some("dependi".to_string()),
        message: "→ Local".to_string(),
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    }
}

/// Create a diagnostic for a deprecated package
fn create_deprecation_diagnostic(dep: &Dependency, version_info: &VersionInfo) -> Diagnostic {
    let mut message = format!(
        "The package '{}' is deprecated. Consider migrating to an alternative.",
        dep.name
    );

    if let Some(latest) = &version_info.latest {
        message.push_str(&format!(
            " Latest version: {} (may not be deprecated).",
            latest
        ));
    }

    let mut related_info = Vec::new();

    if let Some(homepage) = &version_info.homepage {
        related_info.push(DiagnosticRelatedInformation {
            location: Location {
                uri: Url::parse(homepage).unwrap_or_else(|_| {
                    Url::parse("https://example.com").expect("Invalid fallback URL")
                }),
                range: Range::default(),
            },
            message: "Visit package homepage".to_string(),
        });
    }

    if let Some(repo) = &version_info.repository {
        related_info.push(DiagnosticRelatedInformation {
            location: Location {
                uri: Url::parse(repo).unwrap_or_else(|_| {
                    Url::parse("https://github.com").expect("Invalid fallback URL")
                }),
                range: Range::default(),
            },
            message: "View repository for migration guide".to_string(),
        });
    }

    Diagnostic {
        range: Range {
            start: Position {
                line: dep.line,
                character: dep.version_start,
            },
            end: Position {
                line: dep.line,
                character: dep.version_end,
            },
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("deprecated-package".to_string())),
        source: Some("dependi".to_string()),
        message,
        related_information: if related_info.is_empty() {
            None
        } else {
            Some(related_info)
        },
        tags: None,
        code_description: None,
        data: None,
    }
}

/// Create a diagnostic for a yanked version
fn create_yanked_diagnostic(dep: &Dependency, version_info: &VersionInfo) -> Diagnostic {
    let mut message = format!(
        "The version '{}' of '{}' has been yanked from crates.io and should not be used.",
        dep.version, dep.name
    );

    if let Some(latest) = &version_info.latest {
        message.push_str(&format!(" Update to {}.", latest));
    }

    let crates_io_url = format!("https://crates.io/crates/{}", dep.name);
    let mut related_info = vec![DiagnosticRelatedInformation {
        location: Location {
            uri: Url::parse(&crates_io_url)
                .unwrap_or_else(|_| Url::parse("https://crates.io").expect("Invalid fallback URL")),
            range: Range::default(),
        },
        message: "View package on crates.io".to_string(),
    }];

    if let Some(repo) = &version_info.repository {
        related_info.push(DiagnosticRelatedInformation {
            location: Location {
                uri: Url::parse(repo).unwrap_or_else(|_| {
                    Url::parse("https://github.com").expect("Invalid fallback URL")
                }),
                range: Range::default(),
            },
            message: "View repository for more information".to_string(),
        });
    }

    Diagnostic {
        range: Range {
            start: Position {
                line: dep.line,
                character: dep.version_start,
            },
            end: Position {
                line: dep.line,
                character: dep.version_end,
            },
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("yanked-version".to_string())),
        source: Some("dependi".to_string()),
        message,
        related_information: Some(related_info),
        tags: None,
        code_description: Some(CodeDescription {
            href: Url::parse(&crates_io_url)
                .unwrap_or_else(|_| Url::parse("https://crates.io").expect("Invalid fallback URL")),
        }),
        data: None,
    }
}

/// Create a summary diagnostic for multiple vulnerabilities
fn create_vulnerability_summary_diagnostic(
    dep: &Dependency,
    vulns: &[&Vulnerability],
) -> Diagnostic {
    let count = vulns.len();

    // Use the highest severity among all vulnerabilities
    let severity_to_num = |s: &VulnerabilitySeverity| match s {
        VulnerabilitySeverity::Critical => 4,
        VulnerabilitySeverity::High => 3,
        VulnerabilitySeverity::Medium => 2,
        VulnerabilitySeverity::Low => 1,
    };
    let max_severity = vulns
        .iter()
        .map(|v| &v.severity)
        .max_by_key(|s| severity_to_num(s))
        .unwrap_or(&VulnerabilitySeverity::Low);

    let diagnostic_severity = match max_severity {
        VulnerabilitySeverity::Critical | VulnerabilitySeverity::High => DiagnosticSeverity::ERROR,
        VulnerabilitySeverity::Medium => DiagnosticSeverity::WARNING,
        VulnerabilitySeverity::Low => DiagnosticSeverity::HINT,
    };

    // Build summary message
    let vuln_word = if count == 1 { "vuln" } else { "vulns" };
    let vuln_ids: Vec<_> = vulns.iter().map(|v| v.id.as_str()).collect();
    let message = format!("⚠ {} {}: {}", count, vuln_word, vuln_ids.join(", "));

    // Collect related information for all vulnerabilities
    let related_info: Vec<_> = vulns
        .iter()
        .filter_map(|vuln| {
            vuln.url.as_ref().map(|url| DiagnosticRelatedInformation {
                location: Location {
                    uri: Url::parse(url).unwrap_or_else(|_| {
                        Url::parse("https://osv.dev").expect("Invalid fallback URL")
                    }),
                    range: Range::default(),
                },
                message: format!("{}: {}", vuln.id, truncate_string(&vuln.description, 80)),
            })
        })
        .collect();

    Diagnostic {
        range: Range {
            start: Position {
                line: dep.line,
                character: dep.version_start,
            },
            end: Position {
                line: dep.line,
                character: dep.version_end,
            },
        },
        severity: Some(diagnostic_severity),
        code: Some(NumberOrString::String(format!("{}-vulns", count))),
        source: Some("dependi-security".to_string()),
        message,
        related_information: if related_info.is_empty() {
            None
        } else {
            Some(related_info)
        },
        tags: None,
        code_description: vulns.first().and_then(|v| {
            v.url
                .as_ref()
                .and_then(|url| Url::parse(url).ok().map(|href| CodeDescription { href }))
        }),
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{MemoryCache, WriteCache};
    use crate::registries::VersionInfo;

    fn create_test_dependency(name: &str, version: &str, line: u32) -> Dependency {
        Dependency {
            name: name.to_string(),
            version: version.to_string(),
            line,
            name_start: 0,
            name_end: name.len() as u32,
            version_start: name.len() as u32 + 4,
            version_end: name.len() as u32 + 4 + version.len() as u32,
            dev: false,
            optional: false,
        }
    }

    #[test]
    fn test_create_diagnostic_outdated() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("2.0.0"));
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn test_no_diagnostic_up_to_date() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_no_diagnostic_no_cache() {
        let cache = MemoryCache::new();
        let deps = vec![create_test_dependency("unknown", "1.0.0", 5)];
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_severity_filtering() {
        assert!(meets_severity_threshold(
            &VulnerabilitySeverity::Critical,
            &VulnerabilitySeverity::Low
        ));
        assert!(meets_severity_threshold(
            &VulnerabilitySeverity::High,
            &VulnerabilitySeverity::Medium
        ));
        assert!(!meets_severity_threshold(
            &VulnerabilitySeverity::Low,
            &VulnerabilitySeverity::High
        ));
        assert!(meets_severity_threshold(
            &VulnerabilitySeverity::Medium,
            &VulnerabilitySeverity::Medium
        ));
    }

    #[test]
    fn test_deprecated_diagnostic() {
        let deps = vec![create_test_dependency("old-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:old-dep".to_string(),
            VersionInfo {
                deprecated: true,
                latest: Some("2.0.0".to_string()),
                homepage: Some("https://example.com".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let deprecation_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("deprecated")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(deprecation_diags.len(), 1);
        assert!(deprecation_diags[0].message.contains("deprecated"));
        assert_eq!(
            deprecation_diags[0].severity,
            Some(DiagnosticSeverity::WARNING)
        );
        assert!(deprecation_diags[0].related_information.is_some());
    }

    #[test]
    fn test_no_deprecated_diagnostic_for_active() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                deprecated: false,
                latest: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let deprecation_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("deprecated")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(deprecation_diags.len(), 0);
    }

    #[test]
    fn test_deprecated_with_vulnerabilities() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                deprecated: true,
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-2024-1234".to_string(),
                    severity: VulnerabilitySeverity::High,
                    description: "Test vulnerability".to_string(),
                    url: None,
                }],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let deprecation_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("deprecated")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.starts_with("CVE")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(deprecation_diags.len(), 1);
        assert_eq!(
            vuln_diags.len(),
            0,
            "Deprecated packages should not show individual vulnerability diagnostics"
        );
    }

    #[test]
    fn test_yanked_diagnostic() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                yanked_versions: vec!["1.0.0".to_string()],
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let yanked_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("yanked")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(yanked_diags.len(), 1);
        assert!(yanked_diags[0].message.contains("yanked"));
        assert_eq!(yanked_diags[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn test_no_yanked_diagnostic_for_non_yanked() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                yanked_versions: vec!["0.9.0".to_string()],
                latest: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let yanked_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("yanked")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(yanked_diags.len(), 0);
    }

    #[test]
    fn test_yanked_priority_over_deprecated_diagnostic() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                yanked_versions: vec!["1.0.0".to_string()],
                deprecated: true,
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let yanked_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("yanked")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        let deprecated_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("deprecated")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(yanked_diags.len(), 1, "Should show yanked diagnostic");
        assert_eq!(
            deprecated_diags.len(),
            0,
            "Yanked packages should not show deprecated diagnostic"
        );
    }

    #[test]
    fn test_yanked_priority_over_vulnerabilities_diagnostic() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                yanked_versions: vec!["1.0.0".to_string()],
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-2024-1234".to_string(),
                    severity: VulnerabilitySeverity::High,
                    description: "Test vulnerability".to_string(),
                    url: None,
                }],
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let yanked_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.contains("yanked")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .and_then(|c| match c {
                        NumberOrString::String(s) => Some(s.starts_with("CVE")),
                        _ => None,
                    })
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(yanked_diags.len(), 1, "Should show yanked diagnostic");
        assert_eq!(
            vuln_diags.len(),
            0,
            "Yanked packages should not show vulnerability diagnostics"
        );
    }

    #[test]
    fn test_local_dependency_diagnostic() {
        let deps = vec![create_test_dependency("local-crate", "../local", 5)];
        let cache = MemoryCache::new();

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Local"));
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn test_vulnerability_summary_diagnostic() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                vulnerabilities: vec![
                    Vulnerability {
                        id: "CVE-2024-1234".to_string(),
                        severity: VulnerabilitySeverity::High,
                        description: "High severity vulnerability".to_string(),
                        url: Some("https://osv.dev/CVE-2024-1234".to_string()),
                    },
                    Vulnerability {
                        id: "CVE-2024-5678".to_string(),
                        severity: VulnerabilitySeverity::Medium,
                        description: "Medium severity vulnerability".to_string(),
                        url: None,
                    },
                ],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("vulns")))
            })
            .collect();

        assert_eq!(vuln_diags.len(), 1);
        assert!(vuln_diags[0].message.contains("2 vulns"));
        assert_eq!(vuln_diags[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(vuln_diags[0].related_information.is_some());
    }

    #[test]
    fn test_vulnerability_severity_filtering() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                vulnerabilities: vec![
                    Vulnerability {
                        id: "CVE-2024-LOW".to_string(),
                        severity: VulnerabilitySeverity::Low,
                        description: "Low severity".to_string(),
                        url: None,
                    },
                    Vulnerability {
                        id: "CVE-2024-HIGH".to_string(),
                        severity: VulnerabilitySeverity::High,
                        description: "High severity".to_string(),
                        url: None,
                    },
                ],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(
            &deps,
            &cache,
            |name| format!("test:{}", name),
            Some(VulnerabilitySeverity::High),
        );

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("vulns")))
            })
            .collect();

        assert_eq!(vuln_diags.len(), 1);
        assert!(vuln_diags[0].message.contains("1 vuln"));
    }

    #[test]
    fn test_deprecation_diagnostic_with_repository() {
        let deps = vec![create_test_dependency("old-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:old-dep".to_string(),
            VersionInfo {
                deprecated: true,
                latest: Some("2.0.0".to_string()),
                repository: Some("https://github.com/user/old-dep".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let deprecation_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code.as_ref().is_some_and(
                    |c| matches!(c, NumberOrString::String(s) if s.contains("deprecated")),
                )
            })
            .collect();

        assert_eq!(deprecation_diags.len(), 1);
        assert!(deprecation_diags[0].related_information.is_some());
        let related = deprecation_diags[0].related_information.as_ref().unwrap();
        assert!(!related.is_empty());
    }

    #[test]
    fn test_yanked_diagnostic_with_repository() {
        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                yanked_versions: vec!["1.0.0".to_string()],
                latest: Some("2.0.0".to_string()),
                repository: Some("https://github.com/serde-rs/serde".to_string()),
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let yanked_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("yanked")))
            })
            .collect();

        assert_eq!(yanked_diags.len(), 1);
        assert!(yanked_diags[0].related_information.is_some());
        let related = yanked_diags[0].related_information.as_ref().unwrap();
        assert_eq!(related.len(), 2);
    }

    #[test]
    fn test_vulnerability_low_severity_hint() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-2024-LOW".to_string(),
                    severity: VulnerabilitySeverity::Low,
                    description: "Low severity".to_string(),
                    url: None,
                }],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("vulns")))
            })
            .collect();

        assert_eq!(vuln_diags.len(), 1);
        assert_eq!(vuln_diags[0].severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn test_vulnerability_medium_severity_warning() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-2024-MED".to_string(),
                    severity: VulnerabilitySeverity::Medium,
                    description: "Medium severity".to_string(),
                    url: None,
                }],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("vulns")))
            })
            .collect();

        assert_eq!(vuln_diags.len(), 1);
        assert_eq!(vuln_diags[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn test_vulnerability_critical_severity_error() {
        let deps = vec![create_test_dependency("vuln-dep", "1.0.0", 5)];
        let cache = MemoryCache::new();
        cache.insert(
            "test:vuln-dep".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-2024-CRIT".to_string(),
                    severity: VulnerabilitySeverity::Critical,
                    description: "Critical severity".to_string(),
                    url: None,
                }],
                ..Default::default()
            },
        );

        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name), None);

        let vuln_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, NumberOrString::String(s) if s.contains("vulns")))
            })
            .collect();

        assert_eq!(vuln_diags.len(), 1);
        assert_eq!(vuln_diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }
}
