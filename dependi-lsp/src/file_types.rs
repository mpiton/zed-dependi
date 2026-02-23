//! File type detection and ecosystem mapping
//!
//! This module handles detection of dependency file types from URIs
//! and provides mappings to ecosystems and cache keys.

use tower_lsp::lsp_types::Url;

use crate::vulnerabilities::Ecosystem;

/// Supported dependency file types.
///
/// Each variant corresponds to a specific package manager ecosystem
/// and determines which parser and registry client to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    /// Rust packages (Cargo.toml)
    Cargo,
    /// JavaScript/Node.js packages (package.json)
    Npm,
    /// Python packages (requirements.txt, constraints.txt, pyproject.toml)
    Python,
    /// Go modules (go.mod)
    Go,
    /// PHP packages (composer.json)
    Php,
    /// Dart/Flutter packages (pubspec.yaml)
    Dart,
    /// C#/.NET packages (*.csproj)
    Csharp,
    /// Ruby gems (Gemfile)
    Ruby,
}

impl FileType {
    /// Detect the file type from a document URI.
    ///
    /// Returns `Some(FileType)` if the URI matches a known dependency file pattern,
    /// or `None` if the file type is not recognized.
    pub fn detect(uri: &Url) -> Option<Self> {
        let path = uri.path();
        let filename = path.rsplit('/').next().unwrap_or(path);
        if path.ends_with("Cargo.toml") {
            Some(FileType::Cargo)
        } else if path.ends_with("package.json") {
            Some(FileType::Npm)
        } else if filename.ends_with(".txt")
            && (filename.contains("constraints") || filename.contains("requirements"))
            || path.ends_with("pyproject.toml")
        {
            Some(FileType::Python)
        } else if path.ends_with("go.mod") {
            Some(FileType::Go)
        } else if path.ends_with("composer.json") {
            Some(FileType::Php)
        } else if path.ends_with("pubspec.yaml") {
            Some(FileType::Dart)
        } else if path.ends_with(".csproj") {
            Some(FileType::Csharp)
        } else if path.ends_with("Gemfile") {
            Some(FileType::Ruby)
        } else {
            None
        }
    }

    /// Convert to the corresponding vulnerability ecosystem identifier.
    ///
    /// Used for querying the OSV.dev API with the correct ecosystem.
    pub fn to_ecosystem(self) -> Ecosystem {
        match self {
            FileType::Cargo => Ecosystem::CratesIo,
            FileType::Npm => Ecosystem::Npm,
            FileType::Python => Ecosystem::PyPI,
            FileType::Go => Ecosystem::Go,
            FileType::Php => Ecosystem::Packagist,
            FileType::Dart => Ecosystem::Pub,
            FileType::Csharp => Ecosystem::NuGet,
            FileType::Ruby => Ecosystem::RubyGems,
        }
    }

    /// Generate a cache key for a package.
    ///
    /// The cache key includes the registry prefix (e.g., "crates:", "npm:")
    /// to avoid collisions between packages with the same name in different ecosystems.
    pub fn cache_key(self, package_name: &str) -> String {
        match self {
            FileType::Cargo => format!("crates:{}", package_name),
            FileType::Npm => format!("npm:{}", package_name),
            FileType::Python => format!("pypi:{}", package_name),
            FileType::Go => format!("go:{}", package_name),
            FileType::Php => format!("packagist:{}", package_name),
            FileType::Dart => format!("pub:{}", package_name),
            FileType::Csharp => format!("nuget:{}", package_name),
            FileType::Ruby => format!("rubygems:{}", package_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cargo() {
        let uri = Url::parse("file:///project/Cargo.toml").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Cargo));
    }

    #[test]
    fn test_detect_npm() {
        let uri = Url::parse("file:///project/package.json").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Npm));
    }

    #[test]
    fn test_detect_python_requirements() {
        let uri = Url::parse("file:///project/requirements.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));

        let uri = Url::parse("file:///project/requirements-dev.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));

        let uri = Url::parse("file:///project/dev-requirements.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));
    }

    #[test]
    fn test_detect_python_constraints() {
        let uri = Url::parse("file:///project/constraints.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));

        let uri = Url::parse("file:///project/constraints-dev.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));

        let uri = Url::parse("file:///project/dev-constraints.txt").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));
    }

    #[test]
    fn test_no_false_positive_requirements_dir() {
        let uri = Url::parse("file:///project/requirements/notes.txt").unwrap();
        assert_eq!(FileType::detect(&uri), None);

        let uri = Url::parse("file:///project/constraints/readme.txt").unwrap();
        assert_eq!(FileType::detect(&uri), None);
    }

    #[test]
    fn test_detect_pyproject() {
        let uri = Url::parse("file:///project/pyproject.toml").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Python));
    }

    #[test]
    fn test_detect_go() {
        let uri = Url::parse("file:///project/go.mod").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Go));
    }

    #[test]
    fn test_detect_php() {
        let uri = Url::parse("file:///project/composer.json").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Php));
    }

    #[test]
    fn test_detect_dart() {
        let uri = Url::parse("file:///project/pubspec.yaml").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Dart));
    }

    #[test]
    fn test_detect_csharp() {
        let uri = Url::parse("file:///project/MyProject.csproj").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Csharp));
    }

    #[test]
    fn test_detect_ruby() {
        let uri = Url::parse("file:///project/Gemfile").unwrap();
        assert_eq!(FileType::detect(&uri), Some(FileType::Ruby));
    }

    #[test]
    fn test_detect_unknown() {
        let uri = Url::parse("file:///project/unknown.txt").unwrap();
        assert_eq!(FileType::detect(&uri), None);
    }

    #[test]
    fn test_to_ecosystem() {
        assert_eq!(FileType::Cargo.to_ecosystem(), Ecosystem::CratesIo);
        assert_eq!(FileType::Npm.to_ecosystem(), Ecosystem::Npm);
        assert_eq!(FileType::Python.to_ecosystem(), Ecosystem::PyPI);
        assert_eq!(FileType::Go.to_ecosystem(), Ecosystem::Go);
        assert_eq!(FileType::Php.to_ecosystem(), Ecosystem::Packagist);
        assert_eq!(FileType::Dart.to_ecosystem(), Ecosystem::Pub);
        assert_eq!(FileType::Csharp.to_ecosystem(), Ecosystem::NuGet);
        assert_eq!(FileType::Ruby.to_ecosystem(), Ecosystem::RubyGems);
    }

    #[test]
    fn test_cache_key() {
        assert_eq!(FileType::Cargo.cache_key("serde"), "crates:serde");
        assert_eq!(FileType::Npm.cache_key("lodash"), "npm:lodash");
        assert_eq!(FileType::Python.cache_key("requests"), "pypi:requests");
        assert_eq!(
            FileType::Go.cache_key("github.com/gin-gonic/gin"),
            "go:github.com/gin-gonic/gin"
        );
    }
}
