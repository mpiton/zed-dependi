//! Parser for package.json files
//!
//! Uses serde_json for fast parsing with position tracking via byte offset calculation.

use super::{Dependency, Parser};
use serde_json::Value;

/// Parser for npm package.json dependency files
#[derive(Debug, Default)]
pub struct NpmParser;

impl NpmParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for NpmParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let Ok(value) = serde_json::from_str::<Value>(content) else {
            return Vec::new();
        };

        // Pre-compute line start offsets for position calculation
        let line_offsets = compute_line_offsets(content);

        let mut dependencies = Vec::with_capacity(64);

        // Parse all dependency sections
        parse_dependency_section(
            &value,
            "dependencies",
            content,
            &line_offsets,
            false,
            false,
            &mut dependencies,
        );
        parse_dependency_section(
            &value,
            "devDependencies",
            content,
            &line_offsets,
            true,
            false,
            &mut dependencies,
        );
        parse_dependency_section(
            &value,
            "peerDependencies",
            content,
            &line_offsets,
            false,
            true,
            &mut dependencies,
        );
        parse_dependency_section(
            &value,
            "optionalDependencies",
            content,
            &line_offsets,
            false,
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

/// Parse a dependency section (dependencies, devDependencies, etc.)
fn parse_dependency_section(
    root: &Value,
    section_name: &str,
    content: &str,
    line_offsets: &[usize],
    dev: bool,
    optional: bool,
    dependencies: &mut Vec<Dependency>,
) {
    let Some(section) = root.get(section_name) else {
        return;
    };
    let Some(deps_obj) = section.as_object() else {
        return;
    };

    for (name, version_val) in deps_obj {
        let Some(version) = extract_version(version_val) else {
            continue;
        };

        // Find the dependency in the original content for position tracking
        if let Some(dep) =
            find_dependency_position(content, line_offsets, name, &version, dev, optional)
        {
            dependencies.push(dep);
        }
    }
}

/// Extract version string from various npm formats
fn extract_version(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(obj) => {
            // Handle: { "version": "1.0.0" } or complex specs
            obj.get("version")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        }
        _ => None,
    }
}

/// Find the position of a dependency in the content
fn find_dependency_position(
    content: &str,
    line_offsets: &[usize],
    name: &str,
    version: &str,
    dev: bool,
    optional: bool,
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

                let (_, version_start_col) = offset_to_position(version_abs + 1, line_offsets); // +1 to skip opening quote
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
                    optional,
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
        let parser = NpmParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "react": "^18.2.0",
    "lodash": "4.17.21"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let react = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react.version, "^18.2.0");
        assert!(!react.dev);

        let lodash = deps.iter().find(|d| d.name == "lodash").unwrap();
        assert_eq!(lodash.version, "4.17.21");
    }

    #[test]
    fn test_dev_dependencies() {
        let parser = NpmParser::new();
        let content = r#"{
  "devDependencies": {
    "typescript": "^5.0.0",
    "jest": "^29.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        for dep in &deps {
            assert!(dep.dev);
        }
    }

    #[test]
    fn test_multiple_sections() {
        let parser = NpmParser::new();
        let content = r#"{
  "name": "test",
  "dependencies": {
    "express": "^4.18.0"
  },
  "devDependencies": {
    "nodemon": "^3.0.0"
  },
  "peerDependencies": {
    "react": "^18.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);

        let express = deps.iter().find(|d| d.name == "express").unwrap();
        assert!(!express.dev);
        assert!(!express.optional);

        let nodemon = deps.iter().find(|d| d.name == "nodemon").unwrap();
        assert!(nodemon.dev);

        let react = deps.iter().find(|d| d.name == "react").unwrap();
        assert!(react.optional); // peer deps marked as optional
    }

    #[test]
    fn test_scoped_packages() {
        let parser = NpmParser::new();
        let content = r#"{
  "dependencies": {
    "@types/node": "^20.0.0",
    "@babel/core": "^7.22.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);

        let types_node = deps.iter().find(|d| d.name == "@types/node").unwrap();
        assert_eq!(types_node.version, "^20.0.0");

        let babel = deps.iter().find(|d| d.name == "@babel/core").unwrap();
        assert_eq!(babel.version, "^7.22.0");
    }

    #[test]
    fn test_version_ranges() {
        let parser = NpmParser::new();
        let content = r#"{
  "dependencies": {
    "pkg1": "^1.0.0",
    "pkg2": "~2.0.0",
    "pkg3": ">=3.0.0 <4.0.0",
    "pkg4": "1.0.0 - 2.0.0",
    "pkg5": "*"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 5);

        assert_eq!(
            deps.iter().find(|d| d.name == "pkg1").unwrap().version,
            "^1.0.0"
        );
        assert_eq!(
            deps.iter().find(|d| d.name == "pkg3").unwrap().version,
            ">=3.0.0 <4.0.0"
        );
        assert_eq!(deps.iter().find(|d| d.name == "pkg5").unwrap().version, "*");
    }

    #[test]
    fn test_inline_format() {
        let parser = NpmParser::new();
        let content = r#"{"dependencies": {"pkg": "1.0.0"}}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "pkg");
        assert_eq!(deps[0].version, "1.0.0");
    }

    #[test]
    fn test_position_tracking() {
        let parser = NpmParser::new();
        let content = r#"{
  "dependencies": {
    "react": "^18.0.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);

        let react = &deps[0];
        assert_eq!(react.name, "react");
        assert_eq!(react.line, 2); // 0-indexed, so line 3 is index 2
        // Verify positions are within reasonable bounds
        assert!(react.name_start < react.name_end);
        assert!(react.version_start < react.version_end);
    }

    #[test]
    fn test_optional_dependencies() {
        let parser = NpmParser::new();
        let content = r#"{
  "optionalDependencies": {
    "fsevents": "^2.3.0"
  }
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].optional);
        assert!(!deps[0].dev);
    }

    #[test]
    fn test_complex_version_object() {
        let parser = NpmParser::new();
        let content = r#"{
  "dependencies": {
    "simple": "1.0.0",
    "complex": { "version": "2.0.0" }
  }
}"#;
        let deps = parser.parse(content);
        // Only the simple string version should be parsed
        // Complex objects with version field are also supported
        assert!(!deps.is_empty());
        let simple = deps.iter().find(|d| d.name == "simple").unwrap();
        assert_eq!(simple.version, "1.0.0");
    }

    #[test]
    fn test_empty_dependencies() {
        let parser = NpmParser::new();
        let content = r#"{
  "name": "my-app",
  "dependencies": {}
}"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0);
    }

    #[test]
    fn test_invalid_json() {
        let parser = NpmParser::new();
        let content = "not valid json";
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 0);
    }
}
