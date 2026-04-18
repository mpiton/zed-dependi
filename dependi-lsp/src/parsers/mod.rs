//! Parsers for dependency files (Cargo.toml, package.json, etc.)

use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types;

/// Represents a dependency extracted from a manifest file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Package name
    pub name: String,
    /// Version specifier (e.g., "1.0.0", "^1.0", ">=1,<2")
    pub version: String,
    /// Package name span
    pub name_span: Span,
    /// Version string span
    pub version_span: Span,
    /// Whether this is a dev dependency
    pub dev: bool,
    /// Whether this dependency is optional
    pub optional: bool,
    /// Custom registry name (Cargo only, e.g., "kellnr")
    pub registry: Option<String>,
    /// Resolved version from the lock file (e.g., Cargo.lock), if available
    #[serde(default)]
    pub resolved_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Span {
    /// Line number in the file (0-indexed)
    pub line: u32,
    /// Column where the value starts
    pub line_start: u32,
    /// Column where the value ends
    pub line_end: u32,
}

impl Span {
    pub fn contains_lsp_position(&self, position: &lsp_types::Position) -> bool {
        self.line == position.line && (self.line_start..self.line_end).contains(&position.character)
    }
}

impl Dependency {
    /// Returns the resolved version (from lock file) if available,
    /// otherwise falls back to the declared version from the manifest.
    pub fn effective_version(&self) -> &str {
        self.resolved_version.as_deref().unwrap_or(&self.version)
    }
}

/// Trait for parsing dependency files
pub trait Parser: Send + Sync {
    /// Parse the given file content and extract dependencies
    fn parse(&self, content: &str) -> Vec<Dependency>;
}

pub mod cargo;
pub mod cargo_lock;
pub mod composer_lock;
pub mod csharp;
pub mod dart;
pub mod gemfile_lock;
pub mod go;
pub mod go_sum;
pub mod npm;
pub mod npm_lock;
pub mod packages_lock_json;
pub mod php;
pub mod pubspec_lock;
pub mod python;
pub mod python_lock;
pub mod ruby;
