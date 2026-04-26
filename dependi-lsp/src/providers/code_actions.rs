//! Code actions provider for updating dependencies

use tower_lsp::lsp_types::*;

use crate::cache::ReadCache;
use crate::file_types::FileType;
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
    pub fn prefix(&self) -> &'static str {
        match self {
            VersionUpdateType::Major => "⚠ MAJOR",
            VersionUpdateType::Minor => "+ minor",
            VersionUpdateType::Patch => "· patch",
            VersionUpdateType::PreRelease => "* prerelease",
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
    // Multi-char operators first to avoid partial matches
    let version = version
        .strip_prefix("~=")
        .or_else(|| version.strip_prefix("~>"))
        .or_else(|| version.strip_prefix("==="))
        .or_else(|| version.strip_prefix("!="))
        .or_else(|| version.strip_prefix("=="))
        .or_else(|| version.strip_prefix(">="))
        .or_else(|| version.strip_prefix("<="))
        .or_else(|| version.strip_prefix('^'))
        .or_else(|| version.strip_prefix('~'))
        .or_else(|| version.strip_prefix('>'))
        .or_else(|| version.strip_prefix('<'))
        .or_else(|| version.strip_prefix('='))
        .or_else(|| version.strip_prefix('v'))
        .unwrap_or(version)
        .trim();

    let version = version.split(',').next().unwrap_or(version).trim();

    let parts: Vec<&str> = version.split('.').collect();
    match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => version.to_string(),
    }
}

/// Create code actions for dependencies in the given range
#[expect(
    clippy::too_many_arguments,
    reason = "LSP context requires passing doc state, cache, range, ignore list, and workspace metadata together; refactoring into a struct is tracked for a follow-up."
)]
pub async fn create_code_actions(
    dependencies: &[Dependency],
    cache: &impl ReadCache,
    uri: &Url,
    range: Range,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
    ignored: &[String],
    workspace_root: Option<&std::path::Path>,
    current_settings: Option<&str>,
) -> Vec<CodeActionOrCommand> {
    // Three slices with distinct scopes:
    //   - non_ignored:        all non-ignored deps in the file (used for Update-All tally)
    //   - ignore_candidates:  in-range, non-ignored — eligible for the Ignore action
    //                         (property refs CAN still be ignored even though they
    //                         can't safely be Updated)
    //   - update_candidates:  in-range, non-ignored, NOT property reference —
    //                         eligible for individual Update actions
    let non_ignored: Vec<&Dependency> = dependencies
        .iter()
        .filter(|dep| !crate::config::is_package_ignored(&dep.name, ignored))
        .collect();

    let ignore_candidates: Vec<&Dependency> = non_ignored
        .iter()
        .copied()
        .filter(|dep| (range.start.line..=range.end.line).contains(&dep.version_span.line))
        .collect();

    let update_candidates: Vec<&Dependency> = ignore_candidates
        .iter()
        .copied()
        .filter(|dep| !is_property_reference(dep))
        .collect();

    let mut actions: Vec<CodeActionOrCommand> = Vec::new();

    for dep in &update_candidates {
        if let Some(update) = create_update_action(dep, cache, uri, file_type, &cache_key_fn).await
        {
            actions.push(update);
        }
    }

    for dep in &ignore_candidates {
        if let Some(root) = workspace_root
            && let Some(ignore) = create_ignore_action(dep, root, current_settings)
        {
            actions.push(ignore);
        }
    }

    if let Some(update_all) =
        create_update_all_action(&non_ignored, cache, uri, file_type, &cache_key_fn).await
    {
        actions.insert(0, update_all);
    }

    actions
}

/// Whether the manifest text at `version_span` is a Maven property reference
/// (e.g. `${spring.version}`). Replacing the placeholder with a literal would
/// silently break property-driven version management for every other artifact
/// sharing the same property — so the "update version" quick-fix is suppressed.
fn is_property_reference(dep: &Dependency) -> bool {
    dep.version.starts_with("${") && dep.version.ends_with('}')
}

/// Extract Python version operator prefix from a version string (e.g., "~=" from "~=14.3")
fn extract_python_operator(version: &str) -> Option<&str> {
    let operators = ["===", "~=", "==", ">=", "<=", "!=", ">", "<"];
    operators
        .iter()
        .find(|op| version.starts_with(*op))
        .copied()
}

/// Create an "Update to X.Y.Z" code action for a dependency
async fn create_update_action(
    dep: &Dependency,
    cache: &impl ReadCache,
    uri: &Url,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<CodeActionOrCommand> {
    let dep_name = &*dep.name;
    let cache_key = cache_key_fn(dep_name);
    let version_info = cache.get(&cache_key).await?;

    match compare_versions(dep.effective_version(), &version_info) {
        VersionStatus::UpdateAvailable(new_version) => {
            let update_type = compare_update_type(dep.effective_version(), &new_version);
            let new_text = format_version_for_dep(&new_version, file_type, &dep.version);

            let edit = TextEdit {
                range: Range {
                    start: Position {
                        line: dep.version_span.line,
                        character: dep.version_span.line_start,
                    },
                    end: Position {
                        line: dep.version_span.line,
                        character: dep.version_span.line_end,
                    },
                },
                new_text,
            };

            #[expect(
                clippy::disallowed_types,
                reason = "lsp_types requires `std::collections::HashMap`"
            )]
            let mut changes = std::collections::HashMap::new();
            changes.insert(uri.clone(), vec![edit]);

            let title = format!(
                "{}: Update {dep_name} to {new_version}",
                update_type.prefix()
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

/// Create an "Ignore package" code action that appends `dep.name` to the
/// `lsp.dependi.initialization_options.ignore` list in `.zed/settings.json`.
///
/// Returns `None` if `build_ignore_workspace_edit` fails (e.g. malformed
/// existing settings or invalid path); the caller silently omits the action.
fn create_ignore_action(
    dep: &Dependency,
    workspace_root: &std::path::Path,
    current_settings: Option<&str>,
) -> Option<CodeActionOrCommand> {
    let edit = crate::settings_edit::build_ignore_workspace_edit(
        workspace_root,
        &dep.name,
        current_settings,
    )
    .ok()?;

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Ignore package \"{}\"", dep.name),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }))
}

/// Create an "Update All Dependencies" code action when 2+ updates are available
async fn create_update_all_action(
    dependencies: &[&Dependency],
    cache: &impl ReadCache,
    uri: &Url,
    file_type: FileType,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<CodeActionOrCommand> {
    let mut outdated_deps: Vec<(&Dependency, String)> = Vec::new();
    for dep in dependencies
        .iter()
        .copied()
        .filter(|dep| !is_property_reference(dep))
    {
        let cache_key = cache_key_fn(&dep.name);
        if let Some(version_info) = cache.get(&cache_key).await
            && let VersionStatus::UpdateAvailable(new_version) =
                compare_versions(dep.effective_version(), &version_info)
        {
            outdated_deps.push((dep, new_version));
        }
    }

    if outdated_deps.len() < 2 {
        return None;
    }

    let edits: Vec<TextEdit> = outdated_deps
        .iter()
        .map(|(dep, new_version)| {
            let new_text = format_version_for_dep(new_version, file_type, &dep.version);
            TextEdit {
                range: Range {
                    start: Position {
                        line: dep.version_span.line,
                        character: dep.version_span.line_start,
                    },
                    end: Position {
                        line: dep.version_span.line,
                        character: dep.version_span.line_end,
                    },
                },
                new_text,
            }
        })
        .collect();

    #[expect(
        clippy::disallowed_types,
        reason = "lsp_types requires `std::collections::HashMap`"
    )]
    let mut changes = std::collections::HashMap::new();
    changes.insert(uri.clone(), edits);

    let count = outdated_deps.len();
    let title = format!("Update all {count} dependencies");

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
                format!("v{version}")
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
        FileType::Maven => {
            // Maven pom.xml uses plain version strings inside <version>...</version>
            version.to_string()
        }
    }
}

/// Format version for a dependency update, preserving the original operator for Python
fn format_version_for_dep(
    new_version: &str,
    file_type: FileType,
    original_version: &str,
) -> String {
    if file_type == FileType::Python
        && let Some(op) = extract_python_operator(original_version)
    {
        return format!("{op}{}", format_version(new_version, file_type));
    }
    format_version(new_version, file_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{MemoryCache, WriteCache};
    use crate::parsers::Span;
    use crate::registries::VersionInfo;

    fn create_test_dependency(name: &str, version: &str, line: u32) -> Dependency {
        Dependency {
            name: name.to_string(),
            version: version.to_string(),
            name_span: Span {
                line,
                line_start: 0,
                line_end: name.len() as u32,
            },
            version_span: Span {
                line,
                line_start: name.len() as u32 + 4,
                line_end: name.len() as u32 + 4 + version.len() as u32,
            },
            dev: false,
            optional: false,
            registry: None,
            resolved_version: None,
        }
    }

    #[tokio::test]
    async fn test_create_update_action() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("Update serde to 2.0.0"));
                assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[tokio::test]
    async fn test_no_action_when_up_to_date() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

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

    #[tokio::test]
    async fn test_update_all_action_with_multiple_outdated() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        cache
            .insert(
                "test:tokio".to_string(),
                VersionInfo {
                    latest: Some("1.36.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        cache
            .insert(
                "test:reqwest".to_string(),
                VersionInfo {
                    latest: Some("0.12.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

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

    #[tokio::test]
    async fn test_update_all_action_not_shown_for_single_outdated() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        cache
            .insert(
                "test:tokio".to_string(),
                VersionInfo {
                    latest: Some("1.35.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(!action.title.contains("Update all"));
                assert!(action.title.contains("Update serde to 2.0.0"));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[tokio::test]
    async fn test_update_all_action_not_shown_when_all_up_to_date() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        cache
            .insert(
                "test:tokio".to_string(),
                VersionInfo {
                    latest: Some("1.35.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_version_update_type_major() {
        let update_type = compare_update_type("1.0.0", "2.0.0");
        assert_eq!(update_type, VersionUpdateType::Major);
        assert_eq!(update_type.prefix(), "⚠ MAJOR");
        assert!(!update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_minor() {
        let update_type = compare_update_type("1.5.0", "1.6.0");
        assert_eq!(update_type, VersionUpdateType::Minor);
        assert_eq!(update_type.prefix(), "+ minor");
        assert!(update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_patch() {
        let update_type = compare_update_type("1.5.0", "1.5.1");
        assert_eq!(update_type, VersionUpdateType::Patch);
        assert_eq!(update_type.prefix(), "· patch");
        assert!(update_type.is_preferred());
    }

    #[test]
    fn test_version_update_type_prerelease() {
        let update_type = compare_update_type("1.5.0", "1.5.1-alpha.1");
        assert_eq!(update_type, VersionUpdateType::PreRelease);
        assert_eq!(update_type.prefix(), "* prerelease");
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
        // Ruby pessimistic constraint
        assert_eq!(
            compare_update_type("~> 7.0", "8.1.1"),
            VersionUpdateType::Major
        );
        assert_eq!(
            compare_update_type("~> 4.9", "4.9.4"),
            VersionUpdateType::Patch
        );
    }

    #[test]
    fn test_version_update_type_invalid_semver() {
        let update_type = compare_update_type("invalid", "also-invalid");
        assert_eq!(update_type, VersionUpdateType::Patch);
    }

    #[tokio::test]
    async fn test_code_action_title_with_major_update() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("⚠ MAJOR"));
                assert!(action.title.contains("Update serde to 2.0.0"));
                assert_eq!(action.is_preferred, Some(false));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[tokio::test]
    async fn test_code_action_title_with_minor_update() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:tokio".to_string(),
                VersionInfo {
                    latest: Some("1.36.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("+ minor"));
                assert!(action.title.contains("Update tokio to 1.36.0"));
                assert_eq!(action.is_preferred, Some(true));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[tokio::test]
    async fn test_code_action_title_with_patch_update() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:reqwest".to_string(),
                VersionInfo {
                    latest: Some("0.12.1".to_string()),
                    ..Default::default()
                },
            )
            .await;

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("· patch"));
                assert!(action.title.contains("Update reqwest to 0.12.1"));
                assert_eq!(action.is_preferred, Some(true));
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[test]
    fn test_format_version_all_file_types() {
        assert_eq!(format_version("1.0.0", FileType::Cargo), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Npm), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Python), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Go), "v1.0.0");
        assert_eq!(format_version("v1.0.0", FileType::Go), "v1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Php), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Dart), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Csharp), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Ruby), "1.0.0");
        assert_eq!(format_version("1.0.0", FileType::Maven), "1.0.0");
    }

    #[test]
    fn test_normalize_version_with_partial_versions() {
        let normalized = super::normalize_version("1");
        assert_eq!(normalized, "1.0.0");

        let normalized = super::normalize_version("1.2");
        assert_eq!(normalized, "1.2.0");

        let normalized = super::normalize_version("1.2.3");
        assert_eq!(normalized, "1.2.3");
    }

    #[test]
    fn test_normalize_version_with_range_operators() {
        let normalized = super::normalize_version(">=1.0.0, <2.0.0");
        assert_eq!(normalized, "1.0.0");

        let normalized = super::normalize_version("<=1.5.0");
        assert_eq!(normalized, "1.5.0");

        let normalized = super::normalize_version(">1.0.0");
        assert_eq!(normalized, "1.0.0");

        let normalized = super::normalize_version("<2.0.0");
        assert_eq!(normalized, "2.0.0");

        let normalized = super::normalize_version("=1.0.0");
        assert_eq!(normalized, "1.0.0");

        // Python operators
        let normalized = super::normalize_version("~=14.3");
        assert_eq!(normalized, "14.3.0");

        let normalized = super::normalize_version("==2.0.0");
        assert_eq!(normalized, "2.0.0");

        let normalized = super::normalize_version("!=1.0");
        assert_eq!(normalized, "1.0.0");
    }

    #[test]
    fn test_format_version_for_dep_python_compatible_release() {
        // ~= operator should be preserved in replacement
        assert_eq!(
            format_version_for_dep("14.3", FileType::Python, "~=14.2"),
            "~=14.3"
        );
        // == operator preserved
        assert_eq!(
            format_version_for_dep("3.0.0", FileType::Python, "==2.0.0"),
            "==3.0.0"
        );
        // >= operator preserved
        assert_eq!(
            format_version_for_dep("3.0.0", FileType::Python, ">=2.0.0"),
            ">=3.0.0"
        );
        // Non-Python file types are unchanged
        assert_eq!(
            format_version_for_dep("2.0.0", FileType::Cargo, "1.0.0"),
            "2.0.0"
        );
    }

    #[tokio::test]
    async fn test_python_compatible_release_code_action() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:rich".to_string(),
                VersionInfo {
                    latest: Some("14.3.3".to_string()),
                    ..Default::default()
                },
            )
            .await;

        // ~=14.2 with latest 14.3.3: compare_versions returns UpdateAvailable("14.3")
        let deps = vec![create_test_dependency("rich", "~=14.2", 5)];
        let uri = Url::parse("file:///test/requirements.txt").unwrap();
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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Python,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => {
                assert!(action.title.contains("Update rich to 14.3"));
                // Verify the edit replaces with ~=14.3 (operator preserved)
                if let Some(edit) = &action.edit
                    && let Some(changes) = &edit.changes
                {
                    let edits = changes.get(&uri).unwrap();
                    assert_eq!(edits[0].new_text, "~=14.3");
                }
            }
            _ => panic!("Expected CodeAction"),
        }
    }

    #[tokio::test]
    async fn test_python_compatible_release_up_to_date_no_action() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:rich".to_string(),
                VersionInfo {
                    latest: Some("14.3.3".to_string()),
                    ..Default::default()
                },
            )
            .await;

        // ~=14.3 with latest 14.3.3: should be UpToDate, no code action
        let deps = vec![create_test_dependency("rich", "~=14.3", 5)];
        let uri = Url::parse("file:///test/requirements.txt").unwrap();
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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Python,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 0);
    }

    #[tokio::test]
    async fn test_filter_deps_outside_range() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:serde".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;

        let deps = vec![create_test_dependency("serde", "1.0.0", 50)];
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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 0);
    }

    #[tokio::test]
    async fn test_no_action_when_cache_empty() {
        let cache = MemoryCache::new();

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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        assert_eq!(actions.len(), 0);
    }

    #[tokio::test]
    async fn test_update_action_skipped_for_ignored_package() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:lodash".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("lodash", "1.0.0", 5)];
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
        let ignored = vec!["lodash".to_string()];

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &ignored,
            None,
            None,
        )
        .await;

        assert!(
            actions.is_empty(),
            "ignored package should produce no actions"
        );
    }

    #[tokio::test]
    async fn test_update_action_emitted_when_not_ignored() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:react".to_string(),
                VersionInfo {
                    latest: Some("18.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("react", "17.0.0", 5)];
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
        let ignored = vec!["lodash".to_string()];

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &ignored,
            None,
            None,
        )
        .await;

        assert!(
            !actions.is_empty(),
            "non-ignored outdated package should produce action"
        );
    }

    #[tokio::test]
    async fn test_ignore_action_emitted_when_workspace_root_provided() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:lodash".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("lodash", "1.0.0", 5)];
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
        let workspace = std::path::PathBuf::from("/tmp/ws");

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            Some(&workspace),
            None,
        )
        .await;

        // Expect at least 2 actions: Update + Ignore
        assert!(
            actions.len() >= 2,
            "expected Update and Ignore actions, got {}",
            actions.len()
        );

        let titles: Vec<String> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
                _ => None,
            })
            .collect();
        assert!(titles.iter().any(|t| t.contains("Ignore package")));
        assert!(titles.iter().any(|t| t.contains("\"lodash\"")));
    }

    #[tokio::test]
    async fn test_ignore_action_skipped_when_no_workspace_root() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:lodash".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("lodash", "1.0.0", 5)];
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

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            None,
            None,
        )
        .await;

        let titles: Vec<String> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !titles.iter().any(|t| t.contains("Ignore package")),
            "expected no Ignore action without workspace_root"
        );
    }

    #[tokio::test]
    async fn test_ignore_action_emitted_even_when_up_to_date() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:lodash".to_string(),
                VersionInfo {
                    latest: Some("1.0.0".to_string()), // up-to-date
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("lodash", "1.0.0", 5)];
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
        let workspace = std::path::PathBuf::from("/tmp/ws");

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            Some(&workspace),
            None,
        )
        .await;

        let titles: Vec<String> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
                _ => None,
            })
            .collect();
        assert!(
            titles.iter().any(|t| t.contains("Ignore package")),
            "expected Ignore action for up-to-date package"
        );
    }

    #[tokio::test]
    async fn test_property_reference_gets_ignore_action_but_not_update() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:my-pkg".to_string(),
                VersionInfo {
                    latest: Some("2.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        // Create a dep whose version is a Maven-style property reference.
        let dep = create_test_dependency("my-pkg", "${my.version}", 5);
        let deps = vec![dep];
        let uri = Url::parse("file:///test/pom.xml").unwrap();
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
        let workspace = std::path::PathBuf::from("/tmp/ws");

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            Some(&workspace),
            None,
        )
        .await;

        let titles: Vec<String> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
                _ => None,
            })
            .collect();

        // Property reference: NO Update action, but YES Ignore action
        assert!(
            !titles.iter().any(|t| t.contains("Update my-pkg")),
            "property ref must not get Update action"
        );
        assert!(
            titles.iter().any(|t| t.contains("Ignore package")),
            "property ref MUST get Ignore action"
        );
    }

    #[tokio::test]
    async fn test_ignore_action_kind_and_preference() {
        let cache = MemoryCache::new();
        cache
            .insert(
                "test:lodash".to_string(),
                VersionInfo {
                    latest: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            )
            .await;
        let deps = vec![create_test_dependency("lodash", "1.0.0", 5)];
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
        let workspace = std::path::PathBuf::from("/tmp/ws");

        let actions = create_code_actions(
            &deps,
            &cache,
            &uri,
            range,
            FileType::Cargo,
            |name| format!("test:{name}"),
            &[],
            Some(&workspace),
            None,
        )
        .await;

        let ignore = actions
            .iter()
            .find_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) if ca.title.contains("Ignore package") => {
                    Some(ca)
                }
                _ => None,
            })
            .expect("expected Ignore action");

        assert_eq!(ignore.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(ignore.is_preferred, Some(false));
        assert!(ignore.edit.is_some());
    }
}
