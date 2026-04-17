//! Parser for Cargo.toml files using structured TOML parsing

use taplo::dom::node::{Bool, DomNode, Key, Str};
use taplo::dom::{KeyOrIndex, Node};
use taplo::parser::parse;
use taplo::rowan::TextRange;

use super::{Dependency, Parser, Span};

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

        let line_ranges = content
            .split_inclusive('\n')
            .map({
                let mut offset: usize = 0;
                move |line| {
                    let range = TextRange::at((offset as u32).into(), (line.len() as u32).into());
                    offset += line.len();
                    range
                }
            })
            .collect::<Box<[_]>>();
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
                let deps = entries.iter().filter_map(|(name, value)| {
                    parse_dependency(name, value, &line_ranges, is_dev)
                });
                dependencies.extend(deps);
            }

            // Parse table-style dependencies (e.g., [dependencies.reqwest])
            let pattern = format!("{section_name}.*");
            let Ok(matches) = pattern
                .parse::<taplo::dom::Keys>()
                .and_then(|keys| dom.find_all_matches(keys, false))
            else {
                continue;
            };
            let matches = matches.filter_map(|(key_path, node)| {
                // Extract the dependency name from the key path
                let name_key = key_path.iter().filter_map(KeyOrIndex::as_key).next_back()?;

                // For table dependencies, look for the version key
                let table = node.as_table()?;
                let version_node = table.get("version")?;
                let version_str = version_node.as_str()?;

                let package_node = table.get("package");
                let package_str = package_node.as_ref().and_then(Node::as_str);

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

                let TablePositions {
                    name_span,
                    version_span,
                } = find_dependency_positions(&line_ranges, name_key, package_str, version_str)?;

                Some(Dependency {
                    name: package_str
                        .map(Str::value)
                        .unwrap_or_else(|| name_key.value())
                        .to_owned(),
                    version: version_str.value().to_owned(),
                    name_span,
                    version_span,
                    dev: is_dev,
                    optional,
                    registry: registry.map(str::to_owned),
                    resolved_version: None,
                })
            });

            dependencies.extend(matches);
        }

        // Parse workspace.dependencies section
        if let Some(workspace_table) = dom.get("workspace").as_table()
            && let Some(deps_node) = workspace_table.get("dependencies")
            && let Some(deps_table) = deps_node.as_table()
        {
            let entries = deps_table.entries().read();
            let workspace_deps = entries
                .iter()
                .filter_map(|(name, value)| parse_dependency(name, value, &line_ranges, false));
            dependencies.extend(workspace_deps);
        }

        dependencies
    }
}

/// Parse a single dependency from a TOML node
fn parse_dependency(
    name: &Key,
    node: &Node,
    line_spans: &[TextRange],
    is_dev: bool,
) -> Option<Dependency> {
    match node {
        Node::Str(version) => {
            // Simple dependency: name = "1.0.0"
            let TablePositions {
                name_span,
                version_span,
            } = find_dependency_positions(line_spans, name, None, version)?;

            Some(Dependency {
                name: name.value().to_owned(),
                version: version.value().to_owned(),
                name_span,
                version_span,
                dev: is_dev,
                optional: false,
                registry: None,
                resolved_version: None,
            })
        }
        Node::Table(table) => {
            let package_node = table.get("package");
            let package_str = package_node.as_ref().and_then(Node::as_str);

            // Inline table: name = { version = "1.0.0", ... }
            let version_node = table.get("version")?;
            let version_str = version_node.as_str()?;

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
            } = find_dependency_positions(line_spans, name, package_str, version_str)?;

            Some(Dependency {
                name: package_str
                    .map(Str::value)
                    .unwrap_or_else(|| name.value())
                    .to_owned(),
                version: version_str.value().to_owned(),
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

struct TablePositions {
    name_span: Span,
    version_span: Span,
}

/// Find positions for a dependency
/// - simple: `name = "version"`
/// - inline: `name = { version = "1.0.0", package = "...", ... }`
/// - table: `[dependencies.name]` with `version = "x.y.z"` & `package = "..."`
fn find_dependency_positions(
    line_ranges: &[TextRange],
    name: &Key,
    package: Option<&Str>,
    version: &Str,
) -> Option<TablePositions> {
    const SYNTAX_UNAVAILABLE: &str = "syntax unavailable";

    let name_range = package
        .map(|s| s.syntax().expect(SYNTAX_UNAVAILABLE))
        .unwrap_or_else(|| name.syntax().expect(SYNTAX_UNAVAILABLE))
        .text_range();
    let version_range = version.syntax().expect(SYNTAX_UNAVAILABLE).text_range();

    let name_span = find_range_span(line_ranges, name_range)?;
    let version_span = find_range_span(line_ranges, version_range)?;

    Some(TablePositions {
        name_span,
        version_span,
    })
}

fn find_range_span(haystack: &[TextRange], needle: TextRange) -> Option<Span> {
    let line_idx = line_range_position(haystack, needle)?;
    let line_range = haystack[line_idx];
    Some(Span {
        line: line_idx as u32,
        line_start: (needle.start() - line_range.start()).into(),
        line_end: (needle.end() - line_range.start()).into(),
    })
}

fn line_range_position(haystack: &[TextRange], needle: TextRange) -> Option<usize> {
    haystack
        .binary_search_by(|line_range| line_range.ordering(needle))
        .ok()
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
        assert_eq!(deps.len(), 3, "{deps:#?}");

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
        assert_eq!(deps[0].registry.as_deref(), Some("kellnr"));
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
