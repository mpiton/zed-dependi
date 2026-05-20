//! Parser for pnpm `pnpm-workspace.yaml` catalog dependencies.

use super::{Dependency, Parser, Span};

/// Parser for pnpm workspace catalog dependency files.
#[derive(Debug, Default)]
pub struct PnpmWorkspaceParser;

impl PnpmWorkspaceParser {
    /// Creates a new [`PnpmWorkspaceParser`] instance.
    pub fn new() -> Self {
        Self
    }
}

impl Parser for PnpmWorkspaceParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        parse_default_catalog(content)
    }
}

fn parse_default_catalog(content: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    let mut in_catalog = false;
    let mut catalog_indent = 0usize;

    for (line_number, line) in content.lines().enumerate() {
        let trimmed = line.trim();
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

fn parse_catalog_entry(line_number: u32, line: &str) -> Option<Dependency> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim();
    let (name, version) = trimmed.split_once(':')?;
    let name = name.trim();
    let version = version.trim();
    if name.is_empty() || version.is_empty() {
        return None;
    }

    let name_start = indent + trimmed.find(name)?;
    let version_start = line.find(version)?;

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
