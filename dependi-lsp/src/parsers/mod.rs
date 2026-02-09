//! Parsers for dependency files (Cargo.toml, package.json, etc.)

use serde::{Deserialize, Serialize};

/// Represents a dependency extracted from a manifest file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Package name
    pub name: String,
    /// Version specifier (e.g., "1.0.0", "^1.0", ">=1,<2")
    pub version: String,
    /// Line number in the file (0-indexed)
    pub line: u32,
    /// Column where the package name starts
    pub name_start: u32,
    /// Column where the package name ends
    pub name_end: u32,
    /// Column where the version string starts
    pub version_start: u32,
    /// Column where the version string ends
    pub version_end: u32,
    /// Whether this is a dev dependency
    pub dev: bool,
    /// Whether this dependency is optional
    pub optional: bool,
    /// Custom registry name (Cargo only, e.g., "kellnr")
    pub registry: Option<String>,
}

/// Trait for parsing dependency files
pub trait Parser: Send + Sync {
    /// Parse the given file content and extract dependencies
    fn parse(&self, content: &str) -> Vec<Dependency>;
}

pub mod cargo;
pub mod csharp;
pub mod dart;
pub mod go;
pub mod npm;
pub mod php;
pub mod python;
pub mod ruby;
