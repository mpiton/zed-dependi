//! Parser for PHP Composer files (composer.json)
//!
//! Uses serde_json for fast parsing with position tracking via byte offset calculation.

use super::{Dependency, Parser};
use serde_json::Value;

/// Parser for PHP composer.json dependency files
#[derive(Debug, Default)]
pub struct PhpParser;

impl PhpParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for PhpParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let Ok(value) = serde_json::from_str::<Value>(content) else {
            return Vec::new();
        };

        // Pre-compute line start offsets for position calculation
        let line_offsets = compute_line_offsets(content);

        let mut dependencies = Vec::with_capacity(32);

        // Parse require section
        parse_dependency_section(
            &value,
            "require",
            content,
            &line_offsets,
            false,
            &mut dependencies,
        );

        // Parse require-dev section
        parse_dependency_section(
            &value,
            "require-dev",
            content,
            &line_offsets,
            true,
            &mut dependencies,
        );

        dependencies
    }
}

/// Compute byte offsets for each line start (for position calculation)
fn compute_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Convert byte offset to (line, column) - both 0-indexed
fn offset_to_position(offset: usize, line_offsets: &[usize]) -> (u32, u32) {
    let line = line_offsets
        .iter()
        .rposition(|&start| start <= offset)
        .unwrap_or(0);
    let col = offset - line_offsets[line];
    (line as u32, col as u32)
}

/// Parse a dependency section (require or require-dev)
fn parse_dependency_section(
    root: &Value,
    section_name: &str,
    content: &str,
    line_offsets: &[usize],
    dev: bool,
    dependencies: &mut Vec<Dependency>,
) {
    let Some(section) = root.get(section_name) else {
        return;
    };
    let Some(deps_obj) = section.as_object() else {
        return;
    };

    for (name, version_val) in deps_obj {
        // Skip PHP and extensions
        if name == "php" || name.starts_with("ext-") {
            continue;
        }

        let Some(version) = version_val.as_str() else {
            continue;
        };

        // Find the dependency in the original content for position tracking
        if let Some(dep) = find_dependency_position(content, line_offsets, name, version, dev) {
            dependencies.push(dep);
        }
    }
}

/// Find the position of a dependency in the content
fn find_dependency_position(
    content: &str,
    line_offsets: &[usize],
    name: &str,
    version: &str,
    dev: bool,
) -> Option<Dependency> {
    // Search for the quoted name pattern: "name": "version"
    let search_pattern = format!("\"{}\"", name);

    // Find all occurrences and pick the one followed by a colon and the version
    let mut search_start = 0;
    while let Some(name_offset) = content[search_start..].find(&search_pattern) {
        let abs_offset = search_start + name_offset;
        let after_name = abs_offset + search_pattern.len();

        // Check if this looks like a key (followed by colon)
        let rest = &content[after_name..];
        let trimmed = rest.trim_start();

        if trimmed.starts_with(':') {
            // This is a key, now find the version
            if let Some(version_offset) = rest.find(&format!("\"{}\"", version)) {
                let version_abs = after_name + version_offset;

                // Calculate positions
                let (line, name_start_col) = offset_to_position(abs_offset + 1, line_offsets); // +1 to skip opening quote
                let name_end_col = name_start_col + name.len() as u32;

                let (version_line, version_start_col) =
                    offset_to_position(version_abs + 1, line_offsets); // +1 to skip opening quote

                // Ensure name and version are on the same line
                if version_line != line {
                    search_start = abs_offset + 1;
                    continue;
                }

                let version_end_col = version_start_col + version.len() as u32;

                return Some(Dependency {
                    name: name.to_string(),
                    version: version.to_string(),
                    line,
                    name_start: name_start_col,
                    name_end: name_end_col,
                    version_start: version_start_col,
                    version_end: version_end_col,
                    dev,
                    optional: false,
                    registry: None,
                });
            }
        }

        search_start = abs_offset + 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_dependencies() {
        let parser = PhpParser::new();
        let content = r#"
{
    "name": "myproject",
    "require": {
        "php": ">=8.1",
        "laravel/framework": "^10.0",
        "guzzlehttp/guzzle": "^7.0"
    }
}
"#;
        let deps = parser.parse(content);
        // Should have laravel and guzzle, not php
        assert_eq!(deps.len(), 2);

        let laravel = deps.iter().find(|d| d.name.contains("laravel")).unwrap();
        assert_eq!(laravel.version, "^10.0");
        assert!(!laravel.dev);
    }

    #[test]
    fn test_dev_dependencies() {
        let parser = PhpParser::new();
        let content = r#"
{
    "require": {
        "laravel/framework": "^10.0"
    },
    "require-dev": {
        "phpunit/phpunit": "^10.0",
        "mockery/mockery": "^1.5"
    }
}
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);

        let laravel = deps.iter().find(|d| d.name.contains("laravel")).unwrap();
        assert!(!laravel.dev);

        let phpunit = deps.iter().find(|d| d.name.contains("phpunit")).unwrap();
        assert!(phpunit.dev);

        let mockery = deps.iter().find(|d| d.name.contains("mockery")).unwrap();
        assert!(mockery.dev);
    }

    #[test]
    fn test_skip_extensions() {
        let parser = PhpParser::new();
        let content = r#"
{
    "require": {
        "php": ">=8.1",
        "ext-json": "*",
        "ext-mbstring": "*",
        "laravel/framework": "^10.0"
    }
}
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "laravel/framework");
    }

    #[test]
    fn test_version_constraints() {
        let parser = PhpParser::new();
        let content = r#"
{
    "require": {
        "vendor/exact": "1.0.0",
        "vendor/caret": "^1.0",
        "vendor/tilde": "~1.0",
        "vendor/range": ">=1.0 <2.0"
    }
}
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 4);

        let exact = deps.iter().find(|d| d.name.contains("exact")).unwrap();
        assert_eq!(exact.version, "1.0.0");

        let caret = deps.iter().find(|d| d.name.contains("caret")).unwrap();
        assert_eq!(caret.version, "^1.0");
    }

    #[test]
    fn test_version_position() {
        let parser = PhpParser::new();
        let content = r#"{
    "require": {
        "vendor/pkg": "^1.0.0"
    }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);

        let dep = &deps[0];
        assert_eq!(dep.name, "vendor/pkg");
        assert_eq!(dep.version, "^1.0.0");
    }

    #[test]
    fn test_empty_require() {
        let parser = PhpParser::new();
        let content = r#"{
    "require": {}
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0);
    }

    #[test]
    fn test_invalid_json() {
        let parser = PhpParser::new();
        let content = "not valid json";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0);
    }

    #[test]
    fn test_inline_format() {
        let parser = PhpParser::new();
        let content = r#"{"require": {"vendor/pkg": "1.0.0"}}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "vendor/pkg");
        assert_eq!(deps[0].version, "1.0.0");
    }
}
