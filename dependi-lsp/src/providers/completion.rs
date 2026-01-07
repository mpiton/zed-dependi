//! Completion provider for version suggestions

use chrono::{DateTime, Utc};
use tower_lsp::lsp_types::*;

use crate::cache::Cache;
use crate::parsers::Dependency;

/// Format a release date as a human-readable age string
fn format_release_age(released_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(released_at);

    let days = duration.num_days();
    if days < 0 {
        return "just now".to_string();
    }

    if days == 0 {
        let hours = duration.num_hours();
        if hours == 0 {
            let mins = duration.num_minutes();
            if mins < 1 {
                return "just now".to_string();
            }
            return format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" });
        }
        return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
    }

    if days == 1 {
        return "yesterday".to_string();
    }

    if days < 7 {
        return format!("{} days ago", days);
    }

    if days < 30 {
        let weeks = days / 7;
        return format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" });
    }

    if days < 365 {
        let months = days / 30;
        return format!("{} month{} ago", months, if months == 1 { "" } else { "s" });
    }

    let years = days / 365;
    format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
}

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
        .map(|(i, version)| {
            let is_latest = i == 0;
            let release_date = version_info.get_release_date(version);

            // Build detail string with version and optional release age
            let detail = match release_date {
                Some(dt) => {
                    let age = format_release_age(dt);
                    if is_latest {
                        format!("{} [Latest] - {}", version, age)
                    } else {
                        format!("{} - {}", version, age)
                    }
                }
                None => {
                    if is_latest {
                        format!("{} [Latest]", version)
                    } else {
                        format!("Version {}", version)
                    }
                }
            };

            // Build documentation with more details
            let documentation = {
                let mut doc = String::new();
                if is_latest {
                    doc.push_str("**Latest stable version**\n\n");
                }
                if let Some(dt) = release_date {
                    let date_str = dt.format("%Y-%m-%d").to_string();
                    let age = format_release_age(dt);
                    doc.push_str(&format!("Released: {} ({})", date_str, age));
                }
                if doc.is_empty() {
                    None
                } else {
                    Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: doc,
                    }))
                }
            };

            CompletionItem {
                label: version.clone(),
                kind: Some(CompletionItemKind::CONSTANT),
                detail: Some(detail),
                documentation,
                sort_text: Some(format!("{:04}", i)), // Ensures correct ordering
                insert_text: Some(version.clone()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            }
        })
        .collect();

    Some(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryCache;
    use crate::registries::VersionInfo;
    use chrono::Duration;
    use std::collections::HashMap;

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
    fn test_format_release_age_minutes() {
        let now = Utc::now();
        let released = now - Duration::minutes(30);
        let age = format_release_age(released);
        assert_eq!(age, "30 mins ago");
    }

    #[test]
    fn test_format_release_age_hours() {
        let now = Utc::now();
        let released = now - Duration::hours(5);
        let age = format_release_age(released);
        assert_eq!(age, "5 hours ago");
    }

    #[test]
    fn test_format_release_age_yesterday() {
        let now = Utc::now();
        let released = now - Duration::days(1);
        let age = format_release_age(released);
        assert_eq!(age, "yesterday");
    }

    #[test]
    fn test_format_release_age_days() {
        let now = Utc::now();
        let released = now - Duration::days(5);
        let age = format_release_age(released);
        assert_eq!(age, "5 days ago");
    }

    #[test]
    fn test_format_release_age_weeks() {
        let now = Utc::now();
        let released = now - Duration::days(14);
        let age = format_release_age(released);
        assert_eq!(age, "2 weeks ago");
    }

    #[test]
    fn test_format_release_age_months() {
        let now = Utc::now();
        let released = now - Duration::days(60);
        let age = format_release_age(released);
        assert_eq!(age, "2 months ago");
    }

    #[test]
    fn test_format_release_age_years() {
        let now = Utc::now();
        let released = now - Duration::days(400);
        let age = format_release_age(released);
        assert_eq!(age, "1 year ago");
    }

    #[test]
    fn test_format_release_age_just_now() {
        let now = Utc::now();
        let age = format_release_age(now);
        assert_eq!(age, "just now");
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
    fn test_get_completions_with_release_dates() {
        let cache = MemoryCache::new();
        let now = Utc::now();
        let mut release_dates = HashMap::new();
        release_dates.insert("1.0.200".to_string(), now - Duration::days(2));
        release_dates.insert("1.0.199".to_string(), now - Duration::days(10));

        cache.insert(
            "test:serde".to_string(),
            VersionInfo {
                latest: Some("1.0.200".to_string()),
                versions: vec![
                    "1.0.200".to_string(),
                    "1.0.199".to_string(),
                    "1.0.198".to_string(),
                ],
                release_dates,
                ..Default::default()
            },
        );

        let deps = vec![create_test_dependency("serde", "1.0.0", 5)];
        let position = Position {
            line: 5,
            character: 13,
        };

        let completions = get_completions(&deps, position, &cache, |name| format!("test:{}", name));

        assert!(completions.is_some());
        let items = completions.unwrap();
        assert_eq!(items.len(), 3);

        // First item should have [Latest] and release date
        assert!(items[0].detail.as_ref().unwrap().contains("[Latest]"));
        assert!(items[0].detail.as_ref().unwrap().contains("2 days ago"));

        // Second item should have release date but not [Latest]
        assert!(!items[1].detail.as_ref().unwrap().contains("[Latest]"));
        assert!(items[1].detail.as_ref().unwrap().contains("1 week ago"));

        // Third item has no release date
        assert!(!items[2].detail.as_ref().unwrap().contains("[Latest]"));
        assert_eq!(items[2].detail.as_ref().unwrap(), "Version 1.0.198");
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
