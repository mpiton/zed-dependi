//! Completion provider for version suggestions

use tower_lsp::lsp_types::*;

use crate::cache::Cache;
use crate::parsers::Dependency;

/// Get completions for a position in the document
pub fn get_completions(
    dependencies: &[Dependency],
    position: Position,
    cache: &impl Cache,
    cache_key_fn: impl Fn(&str) -> String,
) -> Option<Vec<CompletionItem>> {
    // Find if we're inside a version field
    let dep = dependencies.iter().find(|d| {
        d.line == position.line
            && position.character >= d.version_start
            && position.character <= d.version_end
    })?;

    let cache_key = cache_key_fn(&dep.name);
    let version_info = cache.get(&cache_key)?;

    // Return the last 10 versions as completions
    let items: Vec<CompletionItem> = version_info
        .versions
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, version)| CompletionItem {
            label: version.clone(),
            kind: Some(CompletionItemKind::CONSTANT),
            detail: Some(format!("Version {}", version)),
            documentation: if i == 0 {
                Some(Documentation::String("Latest version".to_string()))
            } else {
                None
            },
            sort_text: Some(format!("{:04}", i)), // Ensures correct ordering
            insert_text: Some(version.clone()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        })
        .collect();

    Some(items)
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
    fn test_get_completions() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.200".to_string()),
                versions: vec![
                    "1.0.200".to_string(),
                    "1.0.199".to_string(),
                    "1.0.198".to_string(),
                ],
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        // Position inside the version field
        let position = Position {
            line: 5,
            character: 13, // Within version_start to version_end
        };

        let completions = get_completions(&deps, position, &cache, |name| format!("test:{}", name));

        assert!(completions.is_some());
        let items = completions.unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "1.0.200");
        assert_eq!(items[1].label, "1.0.199");
    }

    #[test]
    fn test_no_completions_outside_version() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.200".to_string()),
                versions: vec!["1.0.200".to_string()],
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        // Position outside the version field
        let position = Position {
            line: 5,
            character: 0, // At the start, not in version
        };

        let completions = get_completions(&deps, position, &cache, |name| format!("test:{}", name));

        assert!(completions.is_none());
    }

    #[test]
    fn test_no_completions_wrong_line() {
        let cache = MemoryCache::new();
        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.200".to_string()),
                versions: vec!["1.0.200".to_string()],
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let position = Position {
            line: 10, // Wrong line
            character: 13,
        };

        let completions = get_completions(&deps, position, &cache, |name| format!("test:{}", name));

        assert!(completions.is_none());
    }
}
