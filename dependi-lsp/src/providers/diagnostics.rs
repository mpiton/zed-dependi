//! Diagnostics provider for outdated dependencies and vulnerabilities

use tower_lsp::lsp_types::*;

use crate::cache::Cache;
use crate::parsers::Dependency;
use crate::providers::inlay_hints::{VersionStatus, compare_versions};
use crate::registries::{Vulnerability, VulnerabilitySeverity};

/// Create diagnostics for a list of dependencies
pub fn create_diagnostics(
    dependencies: &[Dependency],
    cache: &impl Cache,
    cache_key_fn: impl Fn(&str) -> String,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for dep in dependencies {
        // Add outdated version diagnostic
        if let Some(diag) = create_outdated_diagnostic(dep, cache, &cache_key_fn) {
            diagnostics.push(diag);
        }

        // Add vulnerability diagnostics
        let cache_key = cache_key_fn(&dep.name);
        if let Some(version_info) = cache.get(&cache_key) {
            for vuln in &version_info.vulnerabilities {
                diagnostics.push(create_vulnerability_diagnostic(dep, vuln));
            }
        }
    }

    diagnostics
}

/// Create a diagnostic for an outdated dependency
fn create_outdated_diagnostic(
    dep: &Dependency,
    cache: &impl Cache,
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
            message: format!("Update available: {} → {}", dep.version, new_version),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        }),
        VersionStatus::UpToDate | VersionStatus::Unknown => None,
    }
}

/// Create a diagnostic for a security vulnerability
fn create_vulnerability_diagnostic(dep: &Dependency, vuln: &Vulnerability) -> Diagnostic {
    // Map vulnerability severity to diagnostic severity
    let severity = match vuln.severity {
        VulnerabilitySeverity::Critical | VulnerabilitySeverity::High => DiagnosticSeverity::ERROR,
        VulnerabilitySeverity::Medium => DiagnosticSeverity::WARNING,
        VulnerabilitySeverity::Low => DiagnosticSeverity::HINT,
    };

    let severity_text = match vuln.severity {
        VulnerabilitySeverity::Critical => "CRITICAL",
        VulnerabilitySeverity::High => "HIGH",
        VulnerabilitySeverity::Medium => "MEDIUM",
        VulnerabilitySeverity::Low => "LOW",
    };

    let message = format!(
        "Security vulnerability {} ({}): {}",
        vuln.id,
        severity_text,
        truncate_string(&vuln.description, 150)
    );

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
        severity: Some(severity),
        code: Some(NumberOrString::String(vuln.id.clone())),
        source: Some("dependi-security".to_string()),
        message,
        related_information: vuln.url.as_ref().map(|url| {
            vec![DiagnosticRelatedInformation {
                location: Location {
                    uri: Url::parse(url).unwrap_or_else(|_| {
                        Url::parse("https://osv.dev").expect("Invalid fallback URL")
                    }),
                    range: Range::default(),
                },
                message: "View security advisory".to_string(),
            }]
        }),
        tags: None,
        code_description: vuln
            .url
            .as_ref()
            .and_then(|url| Url::parse(url).ok().map(|href| CodeDescription { href })),
        data: None,
    }
}

/// Truncate a string to max length with ellipsis
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
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
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name));

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
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name));

        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_no_diagnostic_no_cache() {
        let cache = MemoryCache::new();
        let deps = vec![create_test_dependency("unknown", "1.0.0", 5)];
        let diagnostics = create_diagnostics(&deps, &cache, |name| format!("test:{}", name));

        assert_eq!(diagnostics.len(), 0);
    }
}
