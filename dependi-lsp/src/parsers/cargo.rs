//! Parser for Cargo.toml files using structured TOML parsing

use super::{Dependency, Parser, Span};
use taplo::dom::Node;
use taplo::dom::node::{Bool, Str};
use taplo::parser::parse;

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
        let parsed = parse(content);

        // If there are critical parse errors, return empty
        if parsed.errors.iter().any(|e| e.message.contains("expected")) {
            return Vec::new();
        }

        let dom = parsed.into_dom();

        let mut dependencies = Vec::new();

        // Define dependency sections to parse: (section_name, is_dev)
        let sections = [
            ("dependencies", false),
            ("dev-dependencies", true),
            ("build-dependencies", false),
        ];

        for (section_name, is_dev) in sections {
            // Parse regular section dependencies (e.g., [dependencies])
            if let Some(section) = dom.get(section_name).as_table() {
                let entries = section.entries().read();
                for (key, value) in entries.iter() {
                    let name = key.value().to_string();
                    if let Some(dep) = parse_dependency(&name, value, content, is_dev) {
                        dependencies.push(dep);
                    }
                }
            }

            // Parse table-style dependencies (e.g., [dependencies.reqwest])
            let pattern = format!("{section_name}.*");
            if let Ok(keys) = pattern.parse::<taplo::dom::Keys>()
                && let Ok(matches) = dom.find_all_matches(keys, false)
            {
                for (key_path, node) in matches {
                    // Extract the dependency name from the key path
                    let key_str = key_path.to_string();
                    let name = key_str.split('.').next_back().unwrap_or(&key_str);

                    // For table dependencies, look for the version key
                    let Some(table) = node.as_table() else {
                        continue;
                    };
                    let Some(version_node) = table.get("version") else {
                        continue;
                    };
                    let Some(version) = version_node.as_str().map(Str::value) else {
                        continue;
                    };

                    let package_node = table.get("package");
                    let package = package_node.as_ref().and_then(Node::as_str).map(Str::value);

                    let optional = table
                        .get("optional")
                        .as_ref()
                        .and_then(Node::as_bool)
                        .map(Bool::value)
                        .unwrap_or(false);

                    let registry_node = table.get("registry");
                    let registry = registry_node
                        .as_ref()
                        .and_then(Node::as_str)
                        .map(Str::value);

                    let Some(TablePositions {
                        name_span,
                        version_span,
                    }) = find_table_dependency_positions(content, name, package, version)
                    else {
                        continue;
                    };
                    dependencies.push(Dependency {
                        name: package.unwrap_or(name).to_owned(),
                        version: version.to_owned(),
                        name_span,
                        version_span,
                        dev: is_dev,
                        optional,
                        registry: registry.map(str::to_owned),
                        resolved_version: None,
                    });
                }
            }
        }

        // Parse workspace.dependencies section
        if let Some(workspace_table) = dom.get("workspace").as_table()
            && let Some(deps_node) = workspace_table.get("dependencies")
            && let Some(deps_table) = deps_node.as_table()
        {
            let entries = deps_table.entries().read();
            for (key, value) in entries.iter() {
                let name = key.value().to_string();
                if let Some(dep) = parse_dependency(&name, value, content, false) {
                    dependencies.push(dep);
                }
            }
        }

        dependencies
    }
}

/// Parse a single dependency from a TOML node
fn parse_dependency(name: &str, node: &Node, content: &str, is_dev: bool) -> Option<Dependency> {
    match node {
        Node::Str(s) => {
            // Simple dependency: name = "1.0.0"
            let version = s.value().to_string();
            let (line, name_start, name_end, version_start, version_end) =
                find_simple_dependency_positions(content, name, &version)?;

            Some(Dependency {
                name: name.to_string(),
                version,
                name_span: Span {
                    line,
                    line_start: name_start,
                    line_end: name_end,
                },
                version_span: Span {
                    line,
                    line_start: version_start,
                    line_end: version_end,
                },
                dev: is_dev,
                optional: false,
                registry: None,
                resolved_version: None,
            })
        }
        Node::Table(table) => {
            let package_node = table.get("package");
            let package = package_node.as_ref().and_then(Node::as_str).map(Str::value);

            // Inline table: name = { version = "1.0.0", ... }
            let version_node = table.get("version")?;
            let version_str = version_node.as_str()?;
            let version = version_str.value();

            let optional = table
                .get("optional")
                .as_ref()
                .and_then(Node::as_bool)
                .map(Bool::value)
                .unwrap_or(false);

            let registry = table
                .get("registry")
                .as_ref()
                .and_then(Node::as_str)
                .map(|s| s.value().to_owned());

            let TablePositions {
                name_span,
                version_span,
            } = find_inline_table_positions(content, name, package, version)?;

            Some(Dependency {
                name: package.unwrap_or(name).to_owned(),
                version: version.to_owned(),
                name_span,
                version_span,
                dev: is_dev,
                optional,
                registry,
                resolved_version: None,
            })
        }
        _ => None,
    }
}

/// Find positions for a simple dependency: `name = "version"`
fn find_simple_dependency_positions(
    content: &str,
    name: &str,
    version: &str,
) -> Option<(u32, u32, u32, u32, u32)> {
    for (line_idx, line) in content.lines().enumerate() {
        // Look for pattern: name = "version" or name = 'version'
        let trimmed = line.trim();
        if !trimmed.starts_with(name) {
            continue;
        }

        // Check if this line has the exact name followed by =
        let after_name = trimmed[name.len()..].trim_start();
        if !after_name.starts_with('=') {
            continue;
        }

        // Check for simple string value (not inline table)
        let after_eq = after_name[1..].trim_start();
        if after_eq.starts_with('{') {
            continue; // This is an inline table, skip
        }

        // Check if version is in this line
        if !line.contains(version) {
            continue;
        }

        // Calculate positions
        let name_start = line.find(name)? as u32;
        let name_end = name_start + name.len() as u32;

        // Find version position (inside quotes)
        let version_start = line.find(version)? as u32;
        let version_end = version_start + version.len() as u32;

        return Some((
            line_idx as u32,
            name_start,
            name_end,
            version_start,
            version_end,
        ));
    }
    None
}

struct TablePositions {
    name_span: Span,
    version_span: Span,
}

/// Find positions for an inline table dependency: `name = { version = "1.0.0", ... }`
fn find_inline_table_positions(
    content: &str,
    name: &str,
    package: Option<&str>,
    version: &str,
) -> Option<TablePositions> {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with(name) {
            continue;
        }

        // Check if this line has the name followed by = and {
        let after_name = trimmed[name.len()..].trim_start();
        if !after_name.starts_with('=') {
            continue;
        }

        let after_eq = after_name[1..].trim_start();
        if !after_eq.starts_with('{') {
            continue;
        }

        // Check if version is in this line
        if !line.contains(version) {
            continue;
        }

        // Calculate positions
        let (name_start, name_end) = if let Some(package) = package {
            // Find package position (inside quotes after "package =")
            let package_start = line.find(&format!("\"{package}\""))? as u32 + 1;
            let package_end = package_start + package.len() as u32;
            (package_start, package_end)
        } else {
            let name_start = line.find(name)? as u32;
            let name_end = name_start + name.len() as u32;
            (name_start, name_end)
        };

        // Find version position (inside quotes after "version =")
        let version_start = line.find(version)? as u32;
        let version_end = version_start + version.len() as u32;

        let line = line_idx as u32;

        return Some(TablePositions {
            name_span: Span {
                line,
                line_start: name_start,
                line_end: name_end,
            },
            version_span: Span {
                line,
                line_start: version_start,
                line_end: version_end,
            },
        });
    }
    None
}

/// Find positions for a table dependency: `[dependencies.name]` with `version = "x.y.z"`
///
/// For table-style dependencies, the name is in the header `[dependencies.name]`
/// and the version is on a separate line. We return the version line as the primary
/// line since that's what gets highlighted, and set name positions to 0 since the
/// name is on a different line.
fn find_table_dependency_positions(
    content: &str,
    name: &str,
    package: Option<&str>,
    version: &str,
) -> Option<TablePositions> {
    let mut name_span = None::<Span>;
    let mut version_span = None::<Span>;

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Look for the table header containing the dependency name
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.contains(name) {
            // Check if this is a dependencies table
            let inner = &trimmed[1..trimmed.len() - 1];
            if inner.contains("dependencies.") && inner.ends_with(name) {
                let name_start = line.find(name)? as u32;
                let name_end = name_start + name.len() as u32;
                name_span = Some(Span {
                    line: line_idx as u32,
                    line_start: name_start,
                    line_end: name_end,
                });
                continue;
            }
        }

        // If we found the table, look for version = "x.y.z"
        if let Some(name_span) = name_span.as_mut() {
            // Check if we hit a new section
            if trimmed.starts_with('[') {
                break;
            }

            if trimmed.starts_with("version") && line.contains(version) {
                let version_start = line.find(version)? as u32;
                let version_end = version_start + version.len() as u32;

                version_span = Some(Span {
                    line: line_idx as u32,
                    line_start: version_start,
                    line_end: version_end,
                });
            }

            if trimmed.starts_with("package") {
                let package = package?;
                if line.contains(package) {
                    let package_start = line.find(package)? as u32;
                    let package_end = package_start + package.len() as u32;

                    *name_span = Span {
                        line: line_idx as u32,
                        line_start: package_start,
                        line_end: package_end,
                    };
                }
            }
        }
    }

    Some(TablePositions {
        name_span: name_span?,
        version_span: version_span?,
    })
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
        assert_eq!(deps[0].name_span.line_start, 0);
        assert_eq!(deps[0].version, "1.0.0");
    }

    #[test]
    fn test_inline_table_alias_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
serde1 = { package = "serde", version = "1.0.0", features = ["derive"] }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].name_span.line_start, 22);
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
        assert_eq!(deps[0].name_span.line, 1);
        assert_eq!(deps[0].version, "0.12");
    }

    #[test]
    fn test_table_alias_dependency() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies.reqwest1]
package = "reqwest"
version = "0.12"
features = ["json"]
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "reqwest");
        assert_eq!(deps[0].name_span.line, 2);
        assert_eq!(deps[0].name_span.line_start, 11);
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

    #[test]
    fn test_workspace_dependencies_simple() {
        let parser = CargoParser::new();
        let content = r#"
[workspace]
members = ["crate-a"]

[workspace.dependencies]
serde = "1.0.0"
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version, "1.0.0");
    }

    #[test]
    fn test_workspace_dependencies_inline_table() {
        let parser = CargoParser::new();
        let content = r#"
[workspace]
members = ["crate-a"]

[workspace.dependencies]
tokio = { version = "1.0", features = ["full"] }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "tokio");
        assert_eq!(deps[0].version, "1.0");
    }

    #[test]
    fn test_workspace_and_regular_dependencies() {
        let parser = CargoParser::new();
        let content = r#"
[package]
name = "test"
version = "0.1.0"

[workspace]
members = ["crate-a"]

[workspace.dependencies]
serde = "1.0"

[dependencies]
anyhow = "1.0"
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "serde"));
        assert!(deps.iter().any(|d| d.name == "anyhow"));
    }

    #[test]
    fn test_dependency_with_registry() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
my-crate = { version = "0.1.0", registry = "kellnr" }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "my-crate");
        assert_eq!(deps[0].version, "0.1.0");
        assert_eq!(deps[0].registry, Some("kellnr".to_string()));
    }

    #[test]
    fn test_dependency_without_registry() {
        let parser = CargoParser::new();
        let content = r#"
[dependencies]
serde = { version = "1.0.0", features = ["derive"] }
"#;
        let deps = parser.parse(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "serde");
        assert!(deps[0].registry.is_none());
    }
}
