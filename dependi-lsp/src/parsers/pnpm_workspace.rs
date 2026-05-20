//! Parser for pnpm `pnpm-workspace.yaml` catalog dependencies.

use super::{Dependency, Parser, Span};

/// Parser for pnpm workspace catalog dependency files.
#[derive(Debug, Default)]
pub struct PnpmWorkspaceParser;

#[derive(Debug)]
struct NamedCatalog {
    name: String,
    dependencies: Vec<Dependency>,
}

impl PnpmWorkspaceParser {
    /// Creates a new [`PnpmWorkspaceParser`] instance.
    pub fn new() -> Self {
        Self
    }
}

impl Parser for PnpmWorkspaceParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        let mut dependencies = parse_default_catalog(content);
        dependencies.extend(parse_named_catalogs(content));
        dependencies
    }
}

/// Resolve npm `catalog:` dependency references against a pnpm workspace file.
pub fn resolve_catalog_references(
    dependencies: Vec<Dependency>,
    workspace_content: Option<&str>,
) -> Vec<Dependency> {
    let Some(workspace_content) = workspace_content else {
        return dependencies;
    };

    let catalog_dependencies = parse_default_catalog(workspace_content);
    let named_catalogs = parse_named_catalog_collections(workspace_content);

    dependencies
        .into_iter()
        .map(|mut dependency| {
            if dependency.version == "catalog:"
                && let Some(catalog_dependency) = catalog_dependencies
                    .iter()
                    .find(|catalog_dependency| catalog_dependency.name == dependency.name)
            {
                dependency.version = catalog_dependency.version.clone();
            } else if let Some(catalog_name) = dependency.version.strip_prefix("catalog:")
                && let Some(named_catalog) = named_catalogs
                    .iter()
                    .find(|named_catalog| named_catalog.name == catalog_name)
                && let Some(catalog_dependency) = named_catalog
                    .dependencies
                    .iter()
                    .find(|catalog_dependency| catalog_dependency.name == dependency.name)
            {
                dependency.version = catalog_dependency.version.clone();
            }
            dependency
        })
        .collect()
}

fn parse_default_catalog(content: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    let mut in_catalog = false;
    let mut catalog_indent = 0usize;

    for (line_number, line) in content.lines().enumerate() {
        let without_comment = strip_inline_comment(line);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if in_catalog && indent <= catalog_indent {
            in_catalog = false;
        }

        if !in_catalog {
            if trimmed == "catalog:" {
                in_catalog = true;
                catalog_indent = indent;
            }
            continue;
        }

        if let Some(dependency) = parse_catalog_entry(line_number as u32, line) {
            dependencies.push(dependency);
        }
    }

    dependencies
}

fn parse_named_catalogs(content: &str) -> Vec<Dependency> {
    parse_named_catalog_collections(content)
        .into_iter()
        .flat_map(|catalog| catalog.dependencies)
        .collect()
}

fn parse_named_catalog_collections(content: &str) -> Vec<NamedCatalog> {
    let mut catalogs = Vec::new();
    let mut current_catalog: Option<NamedCatalog> = None;
    let mut in_catalogs = false;
    let mut catalogs_indent = 0usize;
    let mut named_catalog_indent = 0usize;

    for (line_number, line) in content.lines().enumerate() {
        let without_comment = strip_inline_comment(line);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if in_catalogs && indent <= catalogs_indent {
            in_catalogs = false;
            if let Some(catalog) = current_catalog.take() {
                catalogs.push(catalog);
            }
        }

        if !in_catalogs {
            if trimmed == "catalogs:" {
                in_catalogs = true;
                catalogs_indent = indent;
            }
            continue;
        }

        if indent <= named_catalog_indent
            && let Some(catalog) = current_catalog.take()
        {
            catalogs.push(catalog);
        }

        if current_catalog.is_none() {
            if let Some(name) = parse_named_catalog_header(trimmed) {
                current_catalog = Some(NamedCatalog {
                    name: name.to_string(),
                    dependencies: Vec::new(),
                });
                named_catalog_indent = indent;
            }
            continue;
        }

        if let Some(dependency) = parse_catalog_entry(line_number as u32, line) {
            current_catalog
                .as_mut()
                .expect("current named catalog")
                .dependencies
                .push(dependency);
        }
    }

    if let Some(catalog) = current_catalog {
        catalogs.push(catalog);
    }

    catalogs
}

fn parse_named_catalog_header(line: &str) -> Option<&str> {
    let (name, value) = line.split_once(':')?;
    let name = name.trim();

    (!name.is_empty() && value.trim().is_empty()).then_some(name)
}

fn parse_catalog_entry(line_number: u32, line: &str) -> Option<Dependency> {
    let indent = line.len() - line.trim_start().len();
    let without_comment = strip_inline_comment(line);
    let trimmed = without_comment.trim();
    let (name, version) = trimmed.split_once(':')?;
    let name = name.trim();
    let raw_version = version.trim();
    let version = trim_quotes(raw_version);
    if name.is_empty() || version.is_empty() {
        return None;
    }

    let name_start = indent + trimmed.find(name)?;
    let raw_version_start = line.find(raw_version)?;
    let quote_offset = raw_version.len() - raw_version.trim_start_matches(['"', '\'']).len();
    let version_start = raw_version_start + quote_offset;

    Some(Dependency {
        name: name.to_string(),
        version: version.to_string(),
        name_span: Span {
            line: line_number,
            line_start: name_start as u32,
            line_end: (name_start + name.len()) as u32,
        },
        version_span: Span {
            line: line_number,
            line_start: version_start as u32,
            line_end: (version_start + version.len()) as u32,
        },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    })
}

fn strip_inline_comment(line: &str) -> &str {
    line.split_once('#')
        .map_or(line, |(before_comment, _)| before_comment)
}

fn trim_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|unquoted| unquoted.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|unquoted| unquoted.strip_suffix('\''))
        })
        .unwrap_or(value)
}
