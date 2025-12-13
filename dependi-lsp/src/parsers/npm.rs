//! Parser for package.json files

use super::{Dependency, Parser};

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
        let mut dependencies = Vec::new();

        // Track which section we're in
        let mut current_section: Option<DependencyType> = None;
        let mut section_brace_depth = 0;
        let mut in_section_object = false;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Check for section headers first
            if let Some(section) = detect_section(trimmed) {
                current_section = Some(section);
                // Count braces after the section name to determine if we're in the object
                let section_start = if trimmed.contains("devDependencies") {
                    trimmed.find("devDependencies").unwrap() + "devDependencies".len()
                } else if trimmed.contains("peerDependencies") {
                    trimmed.find("peerDependencies").unwrap() + "peerDependencies".len()
                } else if trimmed.contains("optionalDependencies") {
                    trimmed.find("optionalDependencies").unwrap() + "optionalDependencies".len()
                } else if trimmed.contains("dependencies") {
                    trimmed.find("dependencies").unwrap() + "dependencies".len()
                } else {
                    0
                };

                let after_section = &trimmed[section_start..];
                if after_section.contains('{') {
                    in_section_object = true;
                    section_brace_depth = 1;

                    // Handle inline dependencies on the same line as section header
                    // e.g., {"dependencies": {"pkg": "1.0.0"}}
                    if let Some(brace_pos) = after_section.find('{') {
                        let deps_content = &after_section[brace_pos + 1..];
                        // Try to parse dependencies from this content
                        for dep in parse_inline_dependencies(deps_content, line_num, section) {
                            dependencies.push(dep);
                        }
                    }
                }
                continue;
            }

            // If we're looking for the opening brace of a section
            if current_section.is_some() && !in_section_object {
                if trimmed.starts_with('{') || trimmed == "{" {
                    in_section_object = true;
                    section_brace_depth = 1;
                    continue;
                }
            }

            // Track brace depth within section
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

            // Parse dependency lines within sections
            if let Some(section) = current_section {
                if in_section_object {
                    if let Some(dep) = parse_dependency_line(line, line_num, section) {
                        dependencies.push(dep);
                    }
                }
            }
        }

        dependencies
    }

    fn file_patterns(&self) -> &[&str] {
        &["package.json"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DependencyType {
    Normal,
    Dev,
    Peer,
    Optional,
}

/// Parse inline dependencies from a single line content
/// e.g., "pkg": "1.0.0", "pkg2": "2.0.0"}}
fn parse_inline_dependencies(content: &str, line_num: u32, dep_type: DependencyType) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut remaining = content;

    while let Some(first_quote) = remaining.find('"') {
        let after_first = &remaining[first_quote + 1..];
        let Some(name_end) = after_first.find('"') else {
            break;
        };
        let name = &after_first[..name_end];

        // Skip if it looks like a section header or closing
        if name.ends_with("ependencies") || name.is_empty() {
            remaining = &after_first[name_end + 1..];
            continue;
        }

        // Find colon and version
        let after_name = &after_first[name_end + 1..];
        let Some(colon_pos) = after_name.find(':') else {
            remaining = after_name;
            continue;
        };

        let after_colon = &after_name[colon_pos + 1..];
        let Some(version_quote_start) = after_colon.find('"') else {
            remaining = after_colon;
            continue;
        };

        let version_content = &after_colon[version_quote_start + 1..];
        let Some(version_end) = version_content.find('"') else {
            remaining = version_content;
            continue;
        };

        let version = &version_content[..version_end];

        deps.push(Dependency {
            name: name.to_string(),
            version: version.to_string(),
            line: line_num,
            name_start: 0, // Approximate for inline
            name_end: 0,
            version_start: 0,
            version_end: 0,
            dev: dep_type == DependencyType::Dev,
            optional: dep_type == DependencyType::Optional || dep_type == DependencyType::Peer,
        });

        remaining = &version_content[version_end + 1..];
    }

    deps
}

fn detect_section(line: &str) -> Option<DependencyType> {
    let line = line.trim();

    if line.contains("\"dependencies\"") && !line.contains("\"devDependencies\"") {
        Some(DependencyType::Normal)
    } else if line.contains("\"devDependencies\"") {
        Some(DependencyType::Dev)
    } else if line.contains("\"peerDependencies\"") {
        Some(DependencyType::Peer)
    } else if line.contains("\"optionalDependencies\"") {
        Some(DependencyType::Optional)
    } else {
        None
    }
}

fn parse_dependency_line(line: &str, line_num: u32, dep_type: DependencyType) -> Option<Dependency> {
    // Match pattern: "package-name": "version"
    // Find the first quoted string (package name)
    let first_quote = line.find('"')?;
    let after_first = &line[first_quote + 1..];
    let name_end = after_first.find('"')?;
    let name = &after_first[..name_end];

    // Skip if it looks like a section header
    if name.ends_with("ependencies") {
        return None;
    }

    // Find the colon
    let colon_pos = line.find(':')?;

    // Find the version string (after the colon)
    let after_colon = &line[colon_pos + 1..];
    let version_first_quote = after_colon.find('"')?;
    let version_start_in_after = version_first_quote + 1;
    let after_version_quote = &after_colon[version_start_in_after..];
    let version_end_in_after = after_version_quote.find('"')?;
    let version = &after_version_quote[..version_end_in_after];

    // Calculate absolute positions
    let name_start = (first_quote + 1) as u32;
    let name_end_pos = name_start + name.len() as u32;

    let version_abs_start = (colon_pos + 1 + version_start_in_after) as u32;
    let version_abs_end = version_abs_start + version.len() as u32;

    Some(Dependency {
        name: name.to_string(),
        version: version.to_string(),
        line: line_num,
        name_start,
        name_end: name_end_pos,
        version_start: version_abs_start,
        version_end: version_abs_end,
        dev: dep_type == DependencyType::Dev,
        optional: dep_type == DependencyType::Optional || dep_type == DependencyType::Peer,
    })
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
}
