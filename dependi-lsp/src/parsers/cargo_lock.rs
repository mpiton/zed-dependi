//! Parser for Cargo.lock files — resolves exact locked versions for Cargo dependencies.

use std::path::{Path, PathBuf};

use hashbrown::HashMap;

/// Parse a Cargo.lock file and return a map of package name → resolved version.
///
/// Cargo.lock uses `[[package]]` TOML array-of-tables entries with `name` and `version` fields.
/// When multiple versions of the same package exist, the first one is kept by default.
///
/// If `root_package` is provided, the root package's `dependencies` list is used to disambiguate:
/// Cargo writes `"crate_name version"` (with version) in dependencies when multiple versions exist,
/// so the version referenced by the root package takes precedence over the first-found version.
pub fn parse_cargo_lock(content: &str, root_package: Option<&str>) -> HashMap<String, String> {
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
            Some(n) => n.to_string(),
            None => continue,
        };
        let version = match pkg.get("version").and_then(|v| v.as_str()) {
            Some(v) => v.to_string(),
            None => continue,
        };

        // Keep the first entry when multiple versions exist
        #[expect(
            clippy::disallowed_methods,
            reason = "`name` is an owned String; `entry_ref` would still allocate on insert"
        )]
        map.entry(name).or_insert(version);
    }

    // Disambiguate multi-version deps using the root package's dependency list.
    // When multiple versions exist, Cargo writes "crate_name version" (with a space) in the
    // dependencies array, allowing us to override the first-found version with the correct one.
    // Old Cargo.lock v1 may use "crate_name version (source_url)" — we strip the source suffix.
    if let Some(root_name) = root_package
        && let Some(root_pkg) = packages
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(root_name))
        && let Some(deps) = root_pkg.get("dependencies").and_then(|d| d.as_array())
    {
        for dep_entry in deps {
            if let Some(dep_str) = dep_entry.as_str() {
                let mut parts = dep_str.splitn(2, ' ');
                if let (Some(crate_name), Some(version_part)) = (parts.next(), parts.next()) {
                    // Strip v1 source suffix: "1.0.0 (registry+...)" → "1.0.0"
                    let version = version_part.split(' ').next().unwrap_or(version_part);
                    map.insert(crate_name.to_string(), version.to_string());
                }
            }
        }
    }

    map
}

/// Find the Cargo.lock file by walking up from a Cargo.toml path.
///
/// Handles both single-crate and workspace layouts by searching parent directories.
/// Uses async I/O to avoid blocking the Tokio executor on slow or networked filesystems.
/// Stops after 10 levels to prevent infinite traversal on unusual file systems.
pub async fn find_cargo_lock(cargo_toml_path: &Path) -> Option<PathBuf> {
    let start_dir = cargo_toml_path.parent()?;

    let mut current = start_dir.to_path_buf();
    let mut depth = 0;
    const MAX_DEPTH: usize = 10;

    loop {
        let candidate = current.join("Cargo.lock");
        if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
            return Some(candidate);
        }

        depth += 1;
        if depth >= MAX_DEPTH {
            return None;
        }

        current = current.parent()?.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_cargo_lock() {
        let content = r#"
version = 3

[[package]]
name = "serde"
version = "1.0.195"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tokio"
version = "1.36.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let map = parse_cargo_lock(content, None);
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.195"));
        assert_eq!(map.get("tokio").map(|s| s.as_str()), Some("1.36.0"));
    }

    #[test]
    fn test_parse_empty_file() {
        let map = parse_cargo_lock("", None);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_no_packages() {
        let content = "version = 3\n";
        let map = parse_cargo_lock(content, None);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_invalid_toml() {
        let map = parse_cargo_lock("not valid toml ][", None);
        assert!(map.is_empty());
    }

    #[test]
    fn test_duplicate_package_keeps_first() {
        let content = r#"
[[package]]
name = "serde"
version = "1.0.100"

[[package]]
name = "serde"
version = "1.0.195"
"#;
        let map = parse_cargo_lock(content, None);
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.100"));
    }

    #[test]
    fn test_multi_version_resolves_root_dependency() {
        let content = r#"
[[package]]
name = "hashbrown"
version = "0.15.5"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "hashbrown"
version = "0.16.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "testing"
version = "0.1.0"
dependencies = [
    "hashbrown 0.16.1",
    "wasip3",
]

[[package]]
name = "wasip3"
version = "0.4.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
    "hashbrown 0.15.5",
]
"#;
        let map = parse_cargo_lock(content, Some("testing"));
        assert_eq!(map.get("hashbrown").map(|s| s.as_str()), Some("0.16.1"));
        assert_eq!(map.get("wasip3").map(|s| s.as_str()), Some("0.4.0"));
    }

    #[test]
    fn test_multi_version_without_root_package_keeps_first() {
        let content = r#"
[[package]]
name = "hashbrown"
version = "0.15.5"

[[package]]
name = "hashbrown"
version = "0.16.1"
"#;
        let map = parse_cargo_lock(content, None);
        assert_eq!(map.get("hashbrown").map(|s| s.as_str()), Some("0.15.5"));
    }

    #[test]
    fn test_root_package_not_found_keeps_first() {
        let content = r#"
[[package]]
name = "hashbrown"
version = "0.15.5"

[[package]]
name = "hashbrown"
version = "0.16.1"
"#;
        let map = parse_cargo_lock(content, Some("nonexistent"));
        assert_eq!(map.get("hashbrown").map(|s| s.as_str()), Some("0.15.5"));
    }

    #[test]
    fn test_unambiguous_dep_not_overridden() {
        let content = r#"
[[package]]
name = "my-crate"
version = "1.0.0"
dependencies = [
    "serde",
]

[[package]]
name = "serde"
version = "1.0.195"
"#;
        let map = parse_cargo_lock(content, Some("my-crate"));
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.195"));
    }

    #[test]
    fn test_v1_source_suffix_stripped() {
        let content = r#"
[[package]]
name = "my-crate"
version = "1.0.0"
dependencies = [
    "serde 1.0.195 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "serde"
version = "1.0.195"
"#;
        let map = parse_cargo_lock(content, Some("my-crate"));
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.195"));
    }
}
