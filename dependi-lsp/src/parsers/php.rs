//! Parser for PHP Composer files (composer.json)

use super::{Dependency, Parser};

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
        let mut dependencies = Vec::new();
        let mut current_section: Option<DependencyType> = None;
        let mut section_brace_depth = 0;
        let mut in_section_object = false;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Detect section start
            if let Some(section) = detect_section(trimmed) {
                current_section = Some(section);
                // Check if the object starts on the same line
                if trimmed.contains('{') {
                    in_section_object = true;
                    section_brace_depth = 1;

                    // Try to parse inline dependencies on the same line
                    if let Some(brace_pos) = trimmed.find('{') {
                        let after_brace = &trimmed[brace_pos + 1..];
                        if let Some(deps) =
                            parse_inline_dependencies(after_brace, line_num, section, line)
                        {
                            dependencies.extend(deps);
                        }
                    }
                }
                continue;
            }

            // Track brace depth for current section
            if in_section_object {
                for ch in trimmed.chars() {
                    match ch {
                        '{' => section_brace_depth += 1,
                        '}' => {
                            section_brace_depth -= 1;
                            if section_brace_depth == 0 {
                                current_section = None;
                                in_section_object = false;
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Skip if not in a dependency section
            let dep_type = match current_section {
                Some(dt) => dt,
                None => continue,
            };

            // Parse dependency line
            if let Some(dep) = parse_dependency_line(line, line_num, dep_type) {
                dependencies.push(dep);
            }
        }

        dependencies
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DependencyType {
    Normal,
    Dev,
}

/// Detect which section we're entering
fn detect_section(line: &str) -> Option<DependencyType> {
    if line.contains("\"require\"") && !line.contains("\"require-dev\"") {
        Some(DependencyType::Normal)
    } else if line.contains("\"require-dev\"") {
        Some(DependencyType::Dev)
    } else {
        None
    }
}

/// Parse inline dependencies (multiple on same line)
fn parse_inline_dependencies(
    content: &str,
    line_num: u32,
    dep_type: DependencyType,
    full_line: &str,
) -> Option<Vec<Dependency>> {
    let mut deps = Vec::new();

    // Simple parsing for inline format: "pkg": "^1.0", "pkg2": "^2.0"
    let parts: Vec<&str> = content.split(',').collect();

    for part in parts {
        if let Some(dep) = parse_dependency_from_pair(part.trim(), line_num, dep_type, full_line) {
            deps.push(dep);
        }
    }

    if deps.is_empty() { None } else { Some(deps) }
}

/// Parse a single dependency line: "vendor/package": "^1.0.0"
fn parse_dependency_line(
    line: &str,
    line_num: u32,
    dep_type: DependencyType,
) -> Option<Dependency> {
    let trimmed = line.trim();

    // Must contain a colon (key: value)
    if !trimmed.contains(':') {
        return None;
    }

    parse_dependency_from_pair(trimmed, line_num, dep_type, line)
}

/// Parse a "name": "version" pair
fn parse_dependency_from_pair(
    pair: &str,
    line_num: u32,
    dep_type: DependencyType,
    full_line: &str,
) -> Option<Dependency> {
    // Find the colon separator
    let colon_pos = pair.find(':')?;

    let name_part = &pair[..colon_pos];
    let version_part = &pair[colon_pos + 1..];

    // Extract name from quotes
    let name = extract_quoted_string(name_part)?;

    // Skip PHP extensions and PHP itself
    if name == "php" || name.starts_with("ext-") {
        return None;
    }

    // Extract version from quotes
    let version = extract_quoted_string(version_part)?;

    // Calculate positions in the original line
    let name_start = full_line.find(&name)? as u32;
    let name_end = name_start + name.len() as u32;
    let version_start = full_line.rfind(&version)? as u32;
    let version_end = version_start + version.len() as u32;

    Some(Dependency {
        name,
        version,
        line: line_num,
        name_start,
        name_end,
        version_start,
        version_end,
        dev: dep_type == DependencyType::Dev,
        optional: false,
    })
}

/// Extract a string value from a quoted string
fn extract_quoted_string(s: &str) -> Option<String> {
    let trimmed = s.trim();

    // Find the first quote
    let start_quote = trimmed.find('"')?;
    let after_quote = &trimmed[start_quote + 1..];

    // Find the closing quote
    let end_quote = after_quote.find('"')?;

    Some(after_quote[..end_quote].to_string())
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
}
