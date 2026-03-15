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

    /// Return the registry URL for a given package name.
    pub fn registry_package_url(&self, name: &str) -> Option<String> {
        match self {
            FileType::Cargo => Some(format!("https://crates.io/crates/{name}")),
            FileType::Npm => Some(format!("https://www.npmjs.com/package/{name}")),
            FileType::Python => Some(format!("https://pypi.org/project/{name}")),
            FileType::Go => Some(format!("https://pkg.go.dev/{name}")),
            FileType::Php => Some(format!("https://packagist.org/packages/{name}")),
            FileType::Dart => Some(format!("https://pub.dev/packages/{name}")),
            FileType::Ruby => Some(format!("https://rubygems.org/gems/{name}")),
            FileType::Csharp => Some(format!("https://www.nuget.org/packages/{name}")),
        }
    }

    /// Return a human-readable registry name for tooltips.
    pub fn registry_name(&self) -> &str {
        match self {
            FileType::Cargo => "crates.io",
            FileType::Npm => "npm",
            FileType::Python => "PyPI",
            FileType::Go => "pkg.go.dev",
            FileType::Php => "Packagist",
            FileType::Dart => "pub.dev",
            FileType::Ruby => "RubyGems",
            FileType::Csharp => "NuGet",
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

    #[test]
    fn test_registry_package_url() {
        assert_eq!(
            FileType::Dart.registry_package_url("http"),
            Some("https://pub.dev/packages/http".to_string())
        );
        assert_eq!(
            FileType::Cargo.registry_package_url("serde"),
            Some("https://crates.io/crates/serde".to_string())
        );
        assert_eq!(
            FileType::Npm.registry_package_url("express"),
            Some("https://www.npmjs.com/package/express".to_string())
        );
        assert_eq!(
            FileType::Python.registry_package_url("requests"),
            Some("https://pypi.org/project/requests".to_string())
        );
        assert_eq!(
            FileType::Go.registry_package_url("github.com/gin-gonic/gin"),
            Some("https://pkg.go.dev/github.com/gin-gonic/gin".to_string())
        );
        assert_eq!(
            FileType::Php.registry_package_url("laravel/framework"),
            Some("https://packagist.org/packages/laravel/framework".to_string())
        );
        assert_eq!(
            FileType::Ruby.registry_package_url("rails"),
            Some("https://rubygems.org/gems/rails".to_string())
        );
        assert_eq!(
            FileType::Csharp.registry_package_url("Newtonsoft.Json"),
            Some("https://www.nuget.org/packages/Newtonsoft.Json".to_string())
        );
    }

    #[test]
    fn test_registry_name() {
        assert_eq!(FileType::Cargo.registry_name(), "crates.io");
        assert_eq!(FileType::Npm.registry_name(), "npm");
        assert_eq!(FileType::Python.registry_name(), "PyPI");
        assert_eq!(FileType::Go.registry_name(), "pkg.go.dev");
        assert_eq!(FileType::Php.registry_name(), "Packagist");
        assert_eq!(FileType::Dart.registry_name(), "pub.dev");
        assert_eq!(FileType::Ruby.registry_name(), "RubyGems");
        assert_eq!(FileType::Csharp.registry_name(), "NuGet");
    }
}
