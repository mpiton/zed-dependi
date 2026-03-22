//! Parser for Python lockfiles — resolves exact locked versions for Python dependencies.
//!
//! Supports:
//! - `poetry.lock` (Poetry — TOML with `[[package]]` entries)
//! - `uv.lock` (uv — TOML with `[[package]]` entries)
//! - `pdm.lock` (PDM — TOML with `[[package]]` entries)
//! - `Pipfile.lock` (Pipenv — JSON with `default`/`develop` sections)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Type of Python lockfile detected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PythonLockfileType {
    /// Poetry's `poetry.lock`
    PoetryLock,
    /// uv's `uv.lock`
    UvLock,
    /// PDM's `pdm.lock`
    PdmLock,
    /// Pipenv's `Pipfile.lock`
    PipfileLock,
}

/// Lockfile candidates in priority order.
const LOCKFILE_CANDIDATES: &[(&str, PythonLockfileType)] = &[
    ("poetry.lock", PythonLockfileType::PoetryLock),
    ("uv.lock", PythonLockfileType::UvLock),
    ("pdm.lock", PythonLockfileType::PdmLock),
    ("Pipfile.lock", PythonLockfileType::PipfileLock),
];

/// Find the Python lockfile by walking up from a manifest path (pyproject.toml or requirements.txt).
///
/// Checks for lockfiles in priority order: poetry.lock > uv.lock > pdm.lock > Pipfile.lock.
/// Uses async I/O to avoid blocking the Tokio executor on slow or networked filesystems.
/// Stops after 10 levels to prevent infinite traversal on unusual file systems.
pub async fn find_python_lockfile(manifest_path: &Path) -> Option<(PathBuf, PythonLockfileType)> {
    let start_dir = manifest_path.parent()?;

    let mut current = start_dir.to_path_buf();
    let mut depth = 0;
    const MAX_DEPTH: usize = 10;

    loop {
        for &(filename, lockfile_type) in LOCKFILE_CANDIDATES {
            let candidate = current.join(filename);
            if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                return Some((candidate, lockfile_type));
            }
        }

        depth += 1;
        if depth >= MAX_DEPTH {
            return None;
        }

        current = current.parent()?.to_path_buf();
    }
}

/// Parse a Python lockfile and return a map of normalized package name → resolved version.
///
/// Package names are normalized per PEP 503 (lowercase, `_`/`.`/`-` → `-`) so that
/// lookups match regardless of how the manifest or lockfile spells the name.
pub fn parse_python_lockfile(
    content: &str,
    lockfile_type: PythonLockfileType,
) -> HashMap<String, String> {
    match lockfile_type {
        // poetry.lock, uv.lock, and pdm.lock all share the same TOML [[package]] structure.
        // If any format diverges in the future, add a dedicated parser here.
        PythonLockfileType::PoetryLock
        | PythonLockfileType::UvLock
        | PythonLockfileType::PdmLock => parse_toml_package_array(content),
        PythonLockfileType::PipfileLock => parse_pipfile_lock(content),
    }
}

// ---------------------------------------------------------------------------
// PEP 503 package name normalization
// ---------------------------------------------------------------------------

/// Normalize a Python package name per PEP 503.
///
/// Lowercases the name and replaces runs of `_`, `.`, and `-` with a single `-`.
pub fn normalize_python_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_was_separator = false;

    for ch in name.chars() {
        match ch {
            '_' | '.' | '-' => {
                if !prev_was_separator && !result.is_empty() {
                    result.push('-');
                    prev_was_separator = true;
                }
            }
            _ => {
                for lower in ch.to_lowercase() {
                    result.push(lower);
                }
                prev_was_separator = false;
            }
        }
    }

    // Strip trailing separator
    if result.ends_with('-') {
        result.pop();
    }

    result
}

// ---------------------------------------------------------------------------
// TOML [[package]] parser (poetry.lock, uv.lock, pdm.lock)
// ---------------------------------------------------------------------------

/// Parse a TOML lockfile with `[[package]]` entries containing `name` and `version` fields.
///
/// This shared implementation works for poetry.lock, uv.lock, and pdm.lock since all three
/// use the same `[[package]]` structure with `name` and `version` string fields.
fn parse_toml_package_array(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let value: toml::Value = match toml::from_str(content) {
        Ok(v) => v,
        Err(_) => return map,
    };

    let packages = match value.get("package").and_then(|p| p.as_array()) {
        Some(pkgs) => pkgs,
        None => return map,
    };

    for pkg in packages {
        let name = match pkg.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let version = match pkg.get("version").and_then(|v| v.as_str()) {
            Some(v) => v.to_string(),
            None => continue,
        };
        let normalized = normalize_python_name(name);
        // Keep the first entry when multiple versions exist
        map.entry(normalized).or_insert(version);
    }

    map
}

// ---------------------------------------------------------------------------
// Pipfile.lock (Pipenv — JSON)
// ---------------------------------------------------------------------------

/// Parse Pipenv's `Pipfile.lock`.
///
/// Structure: JSON object with `default` and `develop` sections. Each maps a package name
/// to an object with a `version` field prefixed by `==` (e.g., `"==2.31.0"`).
fn parse_pipfile_lock(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return map,
    };

    // Extract packages from both default and develop sections
    for section in &["default", "develop"] {
        if let Some(deps) = value.get(*section).and_then(|d| d.as_object()) {
            for (name, dep) in deps {
                if let Some(version_str) = dep.get("version").and_then(|v| v.as_str()) {
                    let version = version_str.strip_prefix("==").unwrap_or(version_str);
                    let normalized = normalize_python_name(name);
                    map.entry(normalized).or_insert_with(|| version.to_string());
                }
            }
        }
    }

    map
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- normalize_python_name ------------------------------------------------

    #[test]
    fn normalize_simple() {
        assert_eq!(normalize_python_name("requests"), "requests");
    }

    #[test]
    fn normalize_uppercase() {
        assert_eq!(normalize_python_name("Flask"), "flask");
    }

    #[test]
    fn normalize_underscores() {
        assert_eq!(
            normalize_python_name("typing_extensions"),
            "typing-extensions"
        );
    }

    #[test]
    fn normalize_dots() {
        assert_eq!(normalize_python_name("zope.interface"), "zope-interface");
    }

    #[test]
    fn normalize_mixed_separators() {
        assert_eq!(normalize_python_name("Foo_Bar.Baz-qux"), "foo-bar-baz-qux");
    }

    #[test]
    fn normalize_consecutive_separators() {
        assert_eq!(normalize_python_name("foo__bar"), "foo-bar");
    }

    #[test]
    fn normalize_empty_string() {
        assert_eq!(normalize_python_name(""), "");
    }

    #[test]
    fn normalize_leading_separator() {
        assert_eq!(normalize_python_name("-foo"), "foo");
    }

    #[test]
    fn normalize_trailing_separator() {
        assert_eq!(normalize_python_name("foo-"), "foo");
    }

    // -- poetry.lock ----------------------------------------------------------

    #[test]
    fn parse_poetry_lock_basic() {
        let content = r#"
[[package]]
name = "requests"
version = "2.31.0"
description = "Python HTTP for Humans."
python-versions = ">=3.7"

[[package]]
name = "Flask"
version = "3.0.2"
description = "A simple framework for building complex web applications."
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PoetryLock);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(map.get("flask").map(String::as_str), Some("3.0.2"));
    }

    #[test]
    fn parse_poetry_lock_with_dependencies() {
        let content = r#"
[[package]]
name = "certifi"
version = "2024.2.2"

[package.dependencies]
urllib3 = ">=1.21.1"

[[package]]
name = "urllib3"
version = "2.1.0"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PoetryLock);
        assert_eq!(map.get("certifi").map(String::as_str), Some("2024.2.2"));
        assert_eq!(map.get("urllib3").map(String::as_str), Some("2.1.0"));
    }

    #[test]
    fn parse_poetry_lock_empty() {
        let map = parse_python_lockfile("", PythonLockfileType::PoetryLock);
        assert!(map.is_empty());
    }

    #[test]
    fn parse_poetry_lock_no_packages() {
        let content = r#"
[metadata]
lock-version = "2.0"
python-versions = "^3.11"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PoetryLock);
        assert!(map.is_empty());
    }

    #[test]
    fn parse_poetry_lock_duplicate_keeps_first() {
        let content = r#"
[[package]]
name = "requests"
version = "2.28.0"

[[package]]
name = "requests"
version = "2.31.0"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PoetryLock);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.28.0"));
    }

    // -- uv.lock --------------------------------------------------------------

    #[test]
    fn parse_uv_lock_basic() {
        let content = r#"
version = 1
requires-python = ">=3.12"

[[package]]
name = "requests"
version = "2.31.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "typing-extensions"
version = "4.9.0"
source = { registry = "https://pypi.org/simple" }
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::UvLock);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(
            map.get("typing-extensions").map(String::as_str),
            Some("4.9.0")
        );
    }

    #[test]
    fn parse_uv_lock_with_sdist_and_wheels() {
        let content = r#"
version = 1

[[package]]
name = "click"
version = "8.1.7"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/click-8.1.7.tar.gz", hash = "sha256:abcdef" }
wheels = [
    { url = "https://example.com/click-8.1.7-py3-none-any.whl", hash = "sha256:123456" },
]
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::UvLock);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn parse_uv_lock_skips_packages_without_version() {
        // Workspace members and the project itself may appear without a version field
        let content = r#"
version = 1

[[package]]
name = "my-project"
source = { virtual = "." }

[[package]]
name = "requests"
version = "2.31.0"
source = { registry = "https://pypi.org/simple" }
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::UvLock);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert!(!map.contains_key("my-project"));
    }

    #[test]
    fn parse_uv_lock_empty() {
        let map = parse_python_lockfile("", PythonLockfileType::UvLock);
        assert!(map.is_empty());
    }

    // -- pdm.lock -------------------------------------------------------------

    #[test]
    fn parse_pdm_lock_basic() {
        let content = r#"
[metadata]
groups = ["default"]
strategy = ["cross_platform"]
lock_version = "4.5.0"

[[package]]
name = "requests"
version = "2.31.0"
requires_python = ">=3.7"
summary = "Python HTTP for Humans."

[[package]]
name = "certifi"
version = "2024.2.2"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PdmLock);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(map.get("certifi").map(String::as_str), Some("2024.2.2"));
    }

    #[test]
    fn parse_pdm_lock_empty() {
        let map = parse_python_lockfile("", PythonLockfileType::PdmLock);
        assert!(map.is_empty());
    }

    #[test]
    fn parse_pdm_lock_metadata_only() {
        let content = r#"
[metadata]
groups = ["default"]
lock_version = "4.5.0"
content_hash = "sha256:abc"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PdmLock);
        assert!(map.is_empty());
    }

    // -- Pipfile.lock ---------------------------------------------------------

    #[test]
    fn parse_pipfile_lock_basic() {
        let content = r#"
{
    "_meta": {
        "hash": {"sha256": "abc"},
        "pipfile-spec": 6,
        "requires": {"python_version": "3.11"}
    },
    "default": {
        "requests": {
            "version": "==2.31.0",
            "hashes": ["sha256:abc"]
        },
        "flask": {
            "version": "==3.0.2",
            "hashes": ["sha256:def"]
        }
    },
    "develop": {}
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(map.get("flask").map(String::as_str), Some("3.0.2"));
    }

    #[test]
    fn parse_pipfile_lock_with_develop() {
        let content = r#"
{
    "_meta": {"hash": {"sha256": "abc"}, "pipfile-spec": 6},
    "default": {
        "requests": {"version": "==2.31.0"}
    },
    "develop": {
        "pytest": {"version": "==8.0.0"},
        "black": {"version": "==24.1.0"}
    }
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(map.get("pytest").map(String::as_str), Some("8.0.0"));
        assert_eq!(map.get("black").map(String::as_str), Some("24.1.0"));
    }

    #[test]
    fn parse_pipfile_lock_strips_equals() {
        let content = r#"
{
    "default": {
        "click": {"version": "==8.1.7"}
    },
    "develop": {}
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn parse_pipfile_lock_without_equals() {
        let content = r#"
{
    "default": {
        "click": {"version": "8.1.7"}
    },
    "develop": {}
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn parse_pipfile_lock_normalizes_names() {
        let content = r#"
{
    "default": {
        "typing_extensions": {"version": "==4.9.0"},
        "Jinja2": {"version": "==3.1.3"}
    },
    "develop": {}
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(
            map.get("typing-extensions").map(String::as_str),
            Some("4.9.0")
        );
        assert_eq!(map.get("jinja2").map(String::as_str), Some("3.1.3"));
    }

    #[test]
    fn parse_pipfile_lock_default_wins_over_develop() {
        // When the same package appears in both sections, the default version is kept
        let content = r#"
{
    "default": {
        "requests": {"version": "==2.31.0"}
    },
    "develop": {
        "requests": {"version": "==2.28.0"}
    }
}
"#;
        let map = parse_pipfile_lock(content);
        assert_eq!(map.get("requests").map(String::as_str), Some("2.31.0"));
    }

    #[test]
    fn parse_pipfile_lock_empty() {
        let map = parse_pipfile_lock("{}");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_pipfile_lock_invalid_json() {
        let map = parse_pipfile_lock("not json");
        assert!(map.is_empty());
    }

    // -- dispatch tests -------------------------------------------------------

    #[test]
    fn dispatch_poetry_lock() {
        let content = r#"
[[package]]
name = "click"
version = "8.1.7"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PoetryLock);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn dispatch_uv_lock() {
        let content = r#"
version = 1
[[package]]
name = "click"
version = "8.1.7"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::UvLock);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn dispatch_pdm_lock() {
        let content = r#"
[[package]]
name = "click"
version = "8.1.7"
"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PdmLock);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    #[test]
    fn dispatch_pipfile_lock() {
        let content = r#"{"default": {"click": {"version": "==8.1.7"}}, "develop": {}}"#;
        let map = parse_python_lockfile(content, PythonLockfileType::PipfileLock);
        assert_eq!(map.get("click").map(String::as_str), Some("8.1.7"));
    }

    // -- cross-format name normalization --------------------------------------

    #[test]
    fn toml_lockfile_normalizes_names() {
        let content = r#"
[[package]]
name = "typing_extensions"
version = "4.9.0"

[[package]]
name = "Jinja2"
version = "3.1.3"

[[package]]
name = "zope.interface"
version = "6.1"
"#;
        let map = parse_toml_package_array(content);
        assert_eq!(
            map.get("typing-extensions").map(String::as_str),
            Some("4.9.0")
        );
        assert_eq!(map.get("jinja2").map(String::as_str), Some("3.1.3"));
        assert_eq!(map.get("zope-interface").map(String::as_str), Some("6.1"));
    }

    #[test]
    fn parse_invalid_toml() {
        let map = parse_toml_package_array("not valid toml ][");
        assert!(map.is_empty());
    }
}
