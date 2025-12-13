//! Parser for Cargo.toml files

use super::{Dependency, Parser};

/// Parser for Rust Cargo.toml dependency files
#[derive(Debug, Default)]
pub struct CargoParser;

impl CargoParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for CargoParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let mut dependencies = Vec::new();
        let mut current_section: Option<DependencySection> = None;
        let mut in_table_dependency: Option<TableDependency> = None;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx as u32;
            let trimmed = line.trim();

            // Check for section headers
            if let Some(section) = parse_section_header(trimmed) {
                // If we were parsing a table dependency, finalize it
                if let Some(table_dep) = in_table_dependency.take() {
                    if let Some(dep) = table_dep.into_dependency() {
                        dependencies.push(dep);
                    }
                }

                // If this is a table dependency like [dependencies.reqwest],
                // we need to track it separately and NOT set current_section
                if let Some(name) = section.table_dependency {
                    let dep_section = section
                        .dependency_section
                        .unwrap_or(DependencySection::Normal);
                    in_table_dependency = Some(TableDependency {
                        name,
                        section: dep_section,
                        version: None,
                        version_line: 0,
                        version_start: 0,
                        version_end: 0,
                        name_line: line_num,
                        name_start: 0,
                        name_end: 0,
                        optional: false,
                    });
                    current_section = None; // Important: don't treat following lines as regular deps
                } else {
                    current_section = section.dependency_section;
                    in_table_dependency = None;
                }
                continue;
            }

            // Skip if not in a dependencies section
            let section = match current_section {
                Some(s) => s,
                None => {
                    // Check if we're in a table dependency section
                    if let Some(ref mut table_dep) = in_table_dependency {
                        if let Some((key, value, value_start, value_end)) = parse_key_value(line) {
                            match key {
                                "version" => {
                                    table_dep.version = Some(unquote(&value));
                                    table_dep.version_line = line_num;
                                    table_dep.version_start = value_start;
                                    table_dep.version_end = value_end;
                                }
                                "optional" => {
                                    table_dep.optional = value.trim() == "true";
                                }
                                _ => {}
                            }
                        }
                    }
                    continue;
                }
            };

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Parse dependency line
            if let Some(dep) = parse_dependency_line(line, line_num, section) {
                dependencies.push(dep);
            }
        }

        // Finalize any remaining table dependency
        if let Some(table_dep) = in_table_dependency {
            if let Some(dep) = table_dep.into_dependency() {
                dependencies.push(dep);
            }
        }

        dependencies
    }

    fn file_patterns(&self) -> &[&str] {
        &["Cargo.toml"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DependencySection {
    Normal,
    Dev,
    Build,
}

struct SectionHeader {
    dependency_section: Option<DependencySection>,
    table_dependency: Option<String>,
}

struct TableDependency {
    name: String,
    section: DependencySection,
    version: Option<String>,
    version_line: u32,
    version_start: u32,
    version_end: u32,
    name_line: u32,
    name_start: u32,
    name_end: u32,
    optional: bool,
}

impl TableDependency {
    fn into_dependency(self) -> Option<Dependency> {
        let version = self.version?;
        Some(Dependency {
            name: self.name,
            version,
            line: self.version_line,
            name_start: self.name_start,
            name_end: self.name_end,
            version_start: self.version_start,
            version_end: self.version_end,
            dev: self.section == DependencySection::Dev,
            optional: self.optional,
        })
    }
}

fn parse_section_header(line: &str) -> Option<SectionHeader> {
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }

    let inner = &line[1..line.len() - 1];

    // Check for table dependency format: [dependencies.package-name]
    if let Some(rest) = inner.strip_prefix("dependencies.") {
        return Some(SectionHeader {
            dependency_section: Some(DependencySection::Normal),
            table_dependency: Some(rest.to_string()),
        });
    }
    if let Some(rest) = inner.strip_prefix("dev-dependencies.") {
        return Some(SectionHeader {
            dependency_section: Some(DependencySection::Dev),
            table_dependency: Some(rest.to_string()),
        });
    }
    if let Some(rest) = inner.strip_prefix("build-dependencies.") {
        return Some(SectionHeader {
            dependency_section: Some(DependencySection::Build),
            table_dependency: Some(rest.to_string()),
        });
    }

    // Regular section headers
    let section = match inner {
        "dependencies" => Some(DependencySection::Normal),
        "dev-dependencies" => Some(DependencySection::Dev),
        "build-dependencies" => Some(DependencySection::Build),
        _ => None,
    };

    Some(SectionHeader {
        dependency_section: section,
        table_dependency: None,
    })
}

fn parse_dependency_line(
    line: &str,
    line_num: u32,
    section: DependencySection,
) -> Option<Dependency> {
    // Find the '=' sign
    let eq_pos = line.find('=')?;

    let name_part = &line[..eq_pos];
    let value_part = &line[eq_pos + 1..];

    let name = name_part.trim();
    if name.is_empty() {
        return None;
    }

    // Calculate name positions
    let name_start = line.find(name)? as u32;
    let name_end = name_start + name.len() as u32;

    // Parse value - can be simple string or inline table
    let value_trimmed = value_part.trim();

    let (version, version_start, version_end, optional) = if value_trimmed.starts_with('{') {
        // Inline table format: { version = "1.0", features = [...] }
        parse_inline_table(line, eq_pos)?
    } else if value_trimmed.starts_with('"') || value_trimmed.starts_with('\'') {
        // Simple string format: "1.0.0"
        let quote_char = value_trimmed.chars().next()?;
        let inner_start = value_trimmed.find(quote_char)? + 1;
        let inner_end = value_trimmed[inner_start..].find(quote_char)?;
        let version = value_trimmed[inner_start..inner_start + inner_end].to_string();

        // Calculate absolute positions
        let abs_start = line.find(value_trimmed)? + inner_start;
        let abs_end = abs_start + inner_end;

        (version, abs_start as u32, abs_end as u32, false)
    } else {
        // Might be a path or git dependency without version
        return None;
    };

    Some(Dependency {
        name: name.to_string(),
        version,
        line: line_num,
        name_start,
        name_end,
        version_start,
        version_end,
        dev: section == DependencySection::Dev,
        optional,
    })
}

fn parse_inline_table(line: &str, eq_pos: usize) -> Option<(String, u32, u32, bool)> {
    let value_part = &line[eq_pos + 1..];

    // Find version in the inline table
    // Look for: version = "x.y.z"
    let version_key = "version";
    let version_pos = value_part.find(version_key)?;

    let after_version_key = &value_part[version_pos + version_key.len()..];
    let eq_in_table = after_version_key.find('=')?;
    let after_eq = &after_version_key[eq_in_table + 1..];

    // Find the quoted version string
    let trimmed = after_eq.trim_start();
    let quote_char = trimmed.chars().next()?;
    if quote_char != '"' && quote_char != '\'' {
        return None;
    }

    let quote_start = after_eq.find(quote_char)?;
    let version_content_start = quote_start + 1;
    let version_content_end = after_eq[version_content_start..].find(quote_char)?;
    let version =
        after_eq[version_content_start..version_content_start + version_content_end].to_string();

    // Calculate absolute positions
    let base_offset = eq_pos + 1 + version_pos + version_key.len() + eq_in_table + 1;
    let abs_start = base_offset + version_content_start;
    let abs_end = abs_start + version_content_end;

    // Check for optional = true
    let optional = value_part.contains("optional")
        && value_part
            .find("optional")
            .and_then(|pos| {
                let after = &value_part[pos..];
                after.find("true")
            })
            .is_some();

    Some((version, abs_start as u32, abs_end as u32, optional))
}

fn parse_key_value(line: &str) -> Option<(&str, String, u32, u32)> {
    let eq_pos = line.find('=')?;
    let key = line[..eq_pos].trim();
    let value_part = &line[eq_pos + 1..];
    let value_trimmed = value_part.trim();

    // Handle quoted strings
    if value_trimmed.starts_with('"') || value_trimmed.starts_with('\'') {
        let quote_char = value_trimmed.chars().next()?;
        let inner_start = 1;
        let inner_end = value_trimmed[inner_start..].find(quote_char)?;
        let value = value_trimmed[inner_start..inner_start + inner_end].to_string();

        let abs_start = line.find(value_trimmed)? + inner_start;
        let abs_end = abs_start + inner_end;

        return Some((key, value, abs_start as u32, abs_end as u32));
    }

    // Handle unquoted values (like booleans)
    let value = value_trimmed.to_string();
    let abs_start = line.find(value_trimmed)? as u32;
    let abs_end = abs_start + value.len() as u32;

    Some((key, value, abs_start, abs_end))
}

fn unquote(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
serde = "1.0.0"
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version, "1.0.0");
        assert!(!deps[0].dev);
    }

    #[test]
    fn test_inline_table_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
serde = { version = "1.0.0", features = ["derive"] }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version, "1.0.0");
    }

    #[test]
    fn test_dev_dependencies() {
        let parser = CargoParser::new();
        let content = r#"
[dev-dependencies]
tokio-test = "0.4"
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "tokio-test");
        assert!(deps[0].dev);
    }

    #[test]
    fn test_multiple_sections() {
        let parser = CargoParser::new();
        let content = r#"
[package]
name = "test"

[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }

[dev-dependencies]
criterion = "0.5"
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 3);

        let serde = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version, "1.0");
        assert!(!serde.dev);

        let tokio = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio.version, "1.0");
        assert!(!tokio.dev);

        let criterion = deps.iter().find(|d| d.name == "criterion").unwrap();
        assert_eq!(criterion.version, "0.5");
        assert!(criterion.dev);
    }

    #[test]
    fn test_table_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies.reqwest]
version = "0.12"
features = ["json"]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "reqwest");
        assert_eq!(deps[0].version, "0.12");
    }

    #[test]
    fn test_optional_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
optional-dep = { version = "1.0", optional = true }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert!(deps[0].optional);
    }
}
