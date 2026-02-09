//! Parser for Cargo.toml files using structured TOML parsing

use super::{Dependency, Parser};
use taplo::dom::Node;
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
            let pattern = format!("{}.*", section_name);
            if let Ok(keys) = pattern.parse::<taplo::dom::Keys>()
                && let Ok(matches) = dom.find_all_matches(keys, false)
            {
                for (key_path, node) in matches {
                    // Extract the dependency name from the key path
                    let key_str = key_path.to_string();
                    let name = key_str
                        .split('.')
                        .next_back()
                        .unwrap_or(&key_str)
                        .to_string();

                    // For table dependencies, look for the version key
                    if let Some(table) = node.as_table()
                        && let Some(version_node) = table.get("version")
                        && let Some(version_str) = version_node.as_str()
                    {
                        let version = version_str.value().to_string();
                        let optional = table
                            .get("optional")
                            .and_then(|n| n.as_bool().map(|b| b.value()))
                            .unwrap_or(false);
                        let registry = table
                            .get("registry")
                            .and_then(|n| n.as_str().map(|s| s.value().to_string()));

                        if let Some((line, name_start, name_end, version_start, version_end)) =
                            find_table_dependency_positions(content, &name, &version)
                        {
                            dependencies.push(Dependency {
                                name,
                                version,
                                line,
                                name_start,
                                name_end,
                                version_start,
                                version_end,
                                dev: is_dev,
                                optional,
                                registry,
                            });
                        }
                    }
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
                line,
                name_start,
                name_end,
                version_start,
                version_end,
                dev: is_dev,
                optional: false,
                registry: None,
            })
        }
        Node::Table(table) => {
            // Inline table: name = { version = "1.0.0", ... }
            let version_node = table.get("version")?;
            let version_str = version_node.as_str()?;
            let version = version_str.value().to_string();

            let optional = table
                .get("optional")
                .and_then(|n| n.as_bool().map(|b| b.value()))
                .unwrap_or(false);

            let registry = table
                .get("registry")
                .and_then(|n| n.as_str().map(|s| s.value().to_string()));

            let (line, name_start, name_end, version_start, version_end) =
                find_inline_table_positions(content, name, &version)?;

            Some(Dependency {
                name: name.to_string(),
                version,
                line,
                name_start,
                name_end,
                version_start,
                version_end,
                dev: is_dev,
                optional,
                registry,
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

/// Find positions for an inline table dependency: `name = { version = "1.0.0", ... }`
fn find_inline_table_positions(
    content: &str,
    name: &str,
    version: &str,
) -> Option<(u32, u32, u32, u32, u32)> {
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
        let name_start = line.find(name)? as u32;
        let name_end = name_start + name.len() as u32;

        // Find version position (inside quotes after "version =")
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

/// Find positions for a table dependency: `[dependencies.name]` with `version = "x.y.z"`
///
/// For table-style dependencies, the name is in the header `[dependencies.name]`
/// and the version is on a separate line. We return the version line as the primary
/// line since that's what gets highlighted, and set name positions to 0 since the
/// name is on a different line.
fn find_table_dependency_positions(
    content: &str,
    name: &str,
    version: &str,
) -> Option<(u32, u32, u32, u32, u32)> {
    let mut found_table = false;

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Look for the table header containing the dependency name
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.contains(name) {
            // Check if this is a dependencies table
            let inner = &trimmed[1..trimmed.len() - 1];
            if inner.contains("dependencies.") && inner.ends_with(name) {
                found_table = true;
                continue;
            }
        }

        // If we found the table, look for version = "x.y.z"
        if found_table {
            // Check if we hit a new section
            if trimmed.starts_with('[') {
                break;
            }

            if trimmed.starts_with("version") && line.contains(version) {
                let version_start = line.find(version)? as u32;
                let version_end = version_start + version.len() as u32;

                // For table dependencies, name is in header (different line)
                // Set name positions to 0 since we can only return one line number
                return Some((line_idx as u32, 0, 0, version_start, version_end));
            }
        }
    }
    None
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
