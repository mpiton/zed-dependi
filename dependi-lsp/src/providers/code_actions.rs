//! Code actions provider for updating dependencies

use std::collections::HashMap;

use tower_lsp::lsp_types::*;

use crate::backend::FileType;
use crate::cache::Cache;
use crate::parsers::Dependency;
use crate::providers::inlay_hints::{VersionStatus, compare_versions};

/// Type of semantic version update
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionUpdateType {
    Major,
    Minor,
    Patch,
    PreRelease,
}

impl VersionUpdateType {
    pub fn label(&self) -> &'static str {
        match self {
            VersionUpdateType::Major => "MAJOR",
            VersionUpdateType::Minor => "minor",
            VersionUpdateType::Patch => "patch",
            VersionUpdateType::PreRelease => "prerelease",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            VersionUpdateType::Major => "🔴",
            VersionUpdateType::Minor => "🟡",
            VersionUpdateType::Patch => "🟢",
            VersionUpdateType::PreRelease => "⚡",
        }
    }

    pub fn is_preferred(&self) -> bool {
        !matches!(self, VersionUpdateType::Major)
    }
}

/// Determine the type of version update between current and new version
pub fn compare_update_type(current: &str, new: &str) -> VersionUpdateType {
    let current_normalized = normalize_version(current);
    let new_normalized = normalize_version(new);

    match (
        semver::Version::parse(&current_normalized),
        semver::Version::parse(&new_normalized),
    ) {
        (Ok(current_ver), Ok(new_ver)) => {
            if !new_ver.pre.is_empty() && current_ver.pre.is_empty() {
                VersionUpdateType::PreRelease
            } else if current_ver.major != new_ver.major {
                VersionUpdateType::Major
            } else if current_ver.minor != new_ver.minor {
                VersionUpdateType::Minor
            } else if current_ver.patch != new_ver.patch {
                VersionUpdateType::Patch
            } else {
                VersionUpdateType::PreRelease
            }
        }
        _ => VersionUpdateType::Patch,
    }
}

fn normalize_version(version: &str) -> String {
    let version = version.trim();
    let version = version
        .strip_prefix('^')
        .or_else(|| version.strip_prefix('~'))
        .or_else(|| version.strip_prefix(">="))
        .or_else(|| version.strip_prefix("<="))
        .or_else(|| version.strip_prefix('>'))
        .or_else(|| version.strip_prefix('<'))
        .or_else(|| version.strip_prefix('='))
        .or_else(|| version.strip_prefix('v'))
        .unwrap_or(version);

    let version = version.split(',').next().unwrap_or(version).trim();

    let parts: Vec<&str> = version.split('.').collect();
    match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => version.to_string(),
    }
}

/// Create code actions for dependencies in the given range
pub fn create_code_actions(
    dependencies: &[Dependency],
    cache: &impl Cache,
    uri: &Url,
    range: Range,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
) -> Vec<CodeActionOrCommand> {
    let mut actions: Vec<CodeActionOrCommand> = dependencies
        .iter()
        .filter(|dep| dep.line >= range.start.line && dep.line <= range.end.line)
        .filter_map(|dep| create_update_action(dep, cache, uri, file_type, &cache_key_fn))
        .collect();

    if let Some(update_all) =
        create_update_all_action(dependencies, cache, uri, file_type, &cache_key_fn)
    {
        actions.insert(0, update_all);
    }

    actions
}

/// Create an "Update to X.Y.Z" code action for a dependency
fn create_update_action(
    dep: &Dependency,
    cache: &impl Cache,
    uri: &Url,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<CodeActionOrCommand> {
    let cache_key = cache_key_fn(&dep.name);
    let version_info = cache.get(&cache_key)?;

    match compare_versions(&dep.version, &version_info) {
        VersionStatus::UpdateAvailable(new_version) => {
            let update_type = compare_update_type(&dep.version, &new_version);
            let new_text = format_version(&new_version, file_type);

            let edit = TextEdit {
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
                new_text,
            };

            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![edit]);

            let title = format!(
                "{} Update {} to {} ({})",
                update_type.emoji(),
                dep.name,
                new_version,
                update_type.label()
            );

            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(update_type.is_preferred()),
                disabled: None,
                data: None,
            }))
        }
        VersionStatus::UpToDate | VersionStatus::Unknown => None,
    }
}

/// Create an "Update All Dependencies" code action when 2+ updates are available
fn create_update_all_action(
    dependencies: &[Dependency],
    cache: &impl Cache,
    uri: &Url,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<CodeActionOrCommand> {
    let outdated_deps: Vec<(&Dependency, String)> = dependencies
        .iter()
        .filter_map(|dep| {
            let cache_key = cache_key_fn(&dep.name);
            let version_info = cache.get(&cache_key)?;

            match compare_versions(&dep.version, &version_info) {
                VersionStatus::UpdateAvailable(new_version) => Some((dep, new_version)),
                _ => None,
            }
        })
        .collect();

    if outdated_deps.len() < 2 {
        return None;
    }

    let edits: Vec<TextEdit> = outdated_deps
        .iter()
        .map(|(dep, new_version)| {
            let new_text = format_version(new_version, file_type);
            TextEdit {
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
                new_text,
            }
        })
        .collect();

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    let count = outdated_deps.len();
    let title = format!("Update all {} dependencies", count);

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }))
}

/// Format version string based on file type
fn format_version(version: &str, file_type: FileType) -> String {
    match file_type {
        FileType::Cargo | FileType::Npm | FileType::Php => {
            // Keep the version as-is - the range already includes the quotes in these formats
            version.to_string()
        }
        FileType::Python => {
            // Python uses operators like == or >=
            // Just replace the version number
            version.to_string()
        }
        FileType::Go => {
            // Go versions start with 'v'
            if version.starts_with('v') {
                version.to_string()
            } else {
                format!("v{}", version)
            }
        }
        FileType::Dart => {
            // Dart pubspec.yaml uses caret syntax (^1.0.0) or simple versions
            version.to_string()
        }
        FileType::Csharp => {
            // C# .csproj uses simple version strings
            version.to_string()
        }
        FileType::Ruby => {
            // Ruby Gemfile uses operators like ~> or >=
            version.to_string()
        }
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
    fn test_create_update_action() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("Update serde to 2.0.0"));
                assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_no_action_when_up_to_date() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_format_version() {
        assert_eq!(format_version("1.0.0", FileType::Cargo), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Npm), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Python), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Go), "v1.0.0");
        assert_eq!(format_version("v1.0.0", FileType::Go), "v1.0.0");
    }

    #[test]
    fn test_update_all_action_with_multiple_outdated() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );
        cache.insert(
            "test:tokio".to_string(),
            VersionInfo {
                latest: Some("1.36.0".to_string()),
                ..Default::default()
            },
        );
        cache.insert(
            "test:reqwest".to_string(),
            VersionInfo {
                latest: Some("0.12.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![
            create_test_dependency("serde", "1.0.0", 5),
            create_test_dependency("tokio", "1.35.0", 6),
            create_test_dependency("reqwest", "0.11.0", 7),
        ];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 4);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("Update all 3 dependencies"));
                assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
                assert_eq!(action.is_preferred, Some(false));

                if let Some(edit) = &action.edit {
                    if let Some(changes) = &edit.changes {
                        let edits = changes.get(&uri).unwrap();
                        assert_eq!(edits.len(), 3);
                    } else {
                        panic!("Expected changes in workspace edit");
                    }
                } else {
                    panic!("Expected edit in code action");
                }
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_update_all_action_not_shown_for_single_outdated() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );
        cache.insert(
            "test:tokio".to_string(),
            VersionInfo {
                latest: Some("1.35.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![
            create_test_dependency("serde", "1.0.0", 5),
            create_test_dependency("tokio", "1.35.0", 6),
        ];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(!action.title.contains("Update all"));
                assert!(action.title.contains("Update serde to 2.0.0"));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_update_all_action_not_shown_when_all_up_to_date() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );
        cache.insert(
            "test:tokio".to_string(),
            VersionInfo {
                latest: Some("1.35.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![
            create_test_dependency("serde", "1.0.0", 5),
            create_test_dependency("tokio", "1.35.0", 6),
        ];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_version_update_type_major() {
        let update_type = compare_update_type("1.0.0", "2.0.0");
        assert_eq!(update_type, VersionUpdateType::Major);
        assert_eq!(update_type.label(), "MAJOR");
        assert_eq!(update_type.emoji(), "🔴");
        assert!(!update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_minor() {
        let update_type = compare_update_type("1.5.0", "1.6.0");
        assert_eq!(update_type, VersionUpdateType::Minor);
        assert_eq!(update_type.label(), "minor");
        assert_eq!(update_type.emoji(), "🟡");
        assert!(update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_patch() {
        let update_type = compare_update_type("1.5.0", "1.5.1");
        assert_eq!(update_type, VersionUpdateType::Patch);
        assert_eq!(update_type.label(), "patch");
        assert_eq!(update_type.emoji(), "🟢");
        assert!(update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_prerelease() {
        let update_type = compare_update_type("1.5.0", "1.5.1-alpha.1");
        assert_eq!(update_type, VersionUpdateType::PreRelease);
        assert_eq!(update_type.label(), "prerelease");
        assert_eq!(update_type.emoji(), "⚡");
        assert!(update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_with_prefixes() {
        assert_eq!(
            compare_update_type("^1.0.0", "2.0.0"),
            VersionUpdateType::Major
        );
        assert_eq!(
            compare_update_type("~1.5.0", "1.6.0"),
            VersionUpdateType::Minor
        );
        assert_eq!(
            compare_update_type(">=1.5.0", "1.5.1"),
            VersionUpdateType::Patch
        );
    }

    #[test]
    fn test_version_update_type_invalid_semver() {
        let update_type = compare_update_type("invalid", "also-invalid");
        assert_eq!(update_type, VersionUpdateType::Patch);
    }

    #[test]
    fn test_code_action_title_with_major_update() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("2.0.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("🔴"));
                assert!(action.title.contains("MAJOR"));
                assert!(action.title.contains("Update serde to 2.0.0"));
                assert_eq!(action.is_preferred, Some(false));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_code_action_title_with_minor_update() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:tokio".to_string(),
            VersionInfo {
                latest: Some("1.36.0".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("tokio", "1.35.0", 5)];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("🟡"));
                assert!(action.title.contains("minor"));
                assert!(action.title.contains("Update tokio to 1.36.0"));
                assert_eq!(action.is_preferred, Some(true));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_code_action_title_with_patch_update() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:reqwest".to_string(),
            VersionInfo {
                latest: Some("0.12.1".to_string()),
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("reqwest", "0.12.0", 5)];
        let uri = Url::parse("file:///test/Cargo.toml").unwrap();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        let actions = create_code_actions(&deps, &cache, &uri, range, FileType::Cargo, |name| {
            format!("test:{}", name)
        });

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("🟢"));
                assert!(action.title.contains("patch"));
                assert!(action.title.contains("Update reqwest to 0.12.1"));
                assert_eq!(action.is_preferred, Some(true));
            }
            _ => panic!("Expected CodeAction"),
        }
    }
}
