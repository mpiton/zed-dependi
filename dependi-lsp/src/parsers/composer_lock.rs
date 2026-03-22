//! Parser for composer.lock files — resolves exact locked versions for PHP (Composer) dependencies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Normalize a Composer package name to lowercase.
///
/// Composer package names are case-insensitive (e.g., "Vendor/Package" == "vendor/package").
/// This function ensures consistent lookup between manifest and lockfile entries.
pub fn normalize_composer_name(name: &str) -> String {
    name.to_lowercase()
}

/// Parse a composer.lock file and return a map of package name → resolved version.
///
/// Composer.lock is a JSON file with `"packages"` and `"packages-dev"` arrays.
/// Each entry has `"name"` and `"version"` fields.
/// Names are normalized to lowercase since Composer is case-insensitive.
/// Versions with a `v` prefix (e.g., "v3.2.0") are stored as-is since both
/// Packagist API and composer.lock use the same format per package.
/// When a package appears multiple times, the first entry is kept.
pub fn parse_composer_lock(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return map,
    };

    for key in &["packages", "packages-dev"] {
        let packages = match value.get(*key).and_then(|p| p.as_array()) {
            Some(pkgs) => pkgs,
            None => continue,
        };

        for pkg in packages {
            let name = match pkg.get("name").and_then(|n| n.as_str()) {
                Some(n) => normalize_composer_name(n),
                None => continue,
            };
            let version = match pkg.get("version").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => continue,
            };
            map.entry(name).or_insert(version);
        }
    }

    map
}

/// Find the composer.lock file by walking up from a composer.json path.
///
/// Handles both single-project and monorepo layouts by searching parent directories.
/// Uses async I/O to avoid blocking the Tokio executor on slow or networked filesystems.
/// Stops after 10 levels to prevent infinite traversal on unusual file systems.
pub async fn find_composer_lock(manifest_path: &Path) -> Option<PathBuf> {
    let start_dir = manifest_path.parent()?;

    let mut current = start_dir.to_path_buf();
    let mut depth = 0;
    const MAX_DEPTH: usize = 10;

    loop {
        let candidate = current.join("composer.lock");
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
    fn test_parse_simple_composer_lock() {
        let content = r#"{
            "packages": [
                {"name": "vendor/package-a", "version": "1.2.3"},
                {"name": "vendor/package-b", "version": "4.5.6"}
            ],
            "packages-dev": []
        }"#;
        let map = parse_composer_lock(content);
        assert_eq!(
            map.get("vendor/package-a").map(|s| s.as_str()),
            Some("1.2.3")
        );
        assert_eq!(
            map.get("vendor/package-b").map(|s| s.as_str()),
            Some("4.5.6")
        );
    }

    #[test]
    fn test_parse_packages_dev() {
        let content = r#"{
            "packages": [
                {"name": "vendor/package-a", "version": "1.0.0"}
            ],
            "packages-dev": [
                {"name": "vendor/dev-tool", "version": "2.0.0"}
            ]
        }"#;
        let map = parse_composer_lock(content);
        assert_eq!(
            map.get("vendor/package-a").map(|s| s.as_str()),
            Some("1.0.0")
        );
        assert_eq!(
            map.get("vendor/dev-tool").map(|s| s.as_str()),
            Some("2.0.0")
        );
    }

    #[test]
    fn test_parse_empty_content() {
        let map = parse_composer_lock("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_empty_arrays() {
        let content = r#"{"packages":[],"packages-dev":[]}"#;
        let map = parse_composer_lock(content);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_invalid_json() {
        let map = parse_composer_lock("not valid json {[");
        assert!(map.is_empty());
    }

    #[test]
    fn test_duplicate_package_keeps_first() {
        let content = r#"{
            "packages": [
                {"name": "vendor/package", "version": "1.0.0"},
                {"name": "vendor/package", "version": "2.0.0"}
            ],
            "packages-dev": []
        }"#;
        let map = parse_composer_lock(content);
        assert_eq!(map.get("vendor/package").map(|s| s.as_str()), Some("1.0.0"));
    }

    #[test]
    fn test_case_insensitive_names() {
        let content = r#"{
            "packages": [
                {"name": "Vendor/Package", "version": "1.0.0"}
            ],
            "packages-dev": []
        }"#;
        let map = parse_composer_lock(content);
        assert!(map.contains_key("vendor/package"));
        assert!(!map.contains_key("Vendor/Package"));
    }

    #[test]
    fn test_various_version_formats() {
        let content = r#"{
            "packages": [
                {"name": "vendor/stable", "version": "1.2.3"},
                {"name": "vendor/prefixed", "version": "v3.2.0"},
                {"name": "vendor/dev", "version": "dev-master"}
            ],
            "packages-dev": []
        }"#;
        let map = parse_composer_lock(content);
        assert_eq!(map.get("vendor/stable").map(|s| s.as_str()), Some("1.2.3"));
        assert_eq!(
            map.get("vendor/prefixed").map(|s| s.as_str()),
            Some("v3.2.0")
        );
        assert_eq!(
            map.get("vendor/dev").map(|s| s.as_str()),
            Some("dev-master")
        );
    }
}
