//! Parser for Cargo.lock files — resolves exact locked versions for Cargo dependencies.

use std::path::{Path, PathBuf};

use hashbrown::HashMap;

/// Parse a Cargo.lock file and return a map of package name → resolved version.
///
/// Cargo.lock uses `[[package]]` TOML array-of-tables entries with `name` and `version` fields.
/// When multiple versions of the same package exist (uncommon but possible), the first one is kept.
pub fn parse_cargo_lock(content: &str) -> HashMap<String, String> {
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
        let map = parse_cargo_lock(content);
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.195"));
        assert_eq!(map.get("tokio").map(|s| s.as_str()), Some("1.36.0"));
    }

    #[test]
    fn test_parse_empty_file() {
        let map = parse_cargo_lock("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_no_packages() {
        let content = "version = 3\n";
        let map = parse_cargo_lock(content);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_invalid_toml() {
        let map = parse_cargo_lock("not valid toml ][");
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
        let map = parse_cargo_lock(content);
        assert_eq!(map.get("serde").map(|s| s.as_str()), Some("1.0.100"));
    }
}
