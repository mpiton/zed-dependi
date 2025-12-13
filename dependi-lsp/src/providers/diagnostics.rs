//! Diagnostics provider for outdated dependencies

use tower_lsp::lsp_types::*;

use crate::cache::Cache;
use crate::parsers::Dependency;
use crate::providers::inlay_hints::{VersionStatus, compare_versions};

/// Create diagnostics for a list of dependencies
pub fn create_diagnostics(
    dependencies: &[Dependency],
    cache: &impl Cache,
    cache_key_fn: impl Fn(&str) -> String,
) -> Vec<Diagnostic> {
    dependencies
        .iter()
        .filter_map(|dep| create_diagnostic_for_dependency(dep, cache, &cache_key_fn))
        .collect()
}

/// Create a diagnostic for a single dependency if it's outdated
fn create_diagnostic_for_dependency(
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
