//! Parser for Go sum files (go.sum) — resolves exact locked versions for Go dependencies.

use std::path::{Path, PathBuf};

use hashbrown::HashMap;

/// Parse a go.sum file and return a map of module path → all observed versions.
///
/// go.sum format: `<module> <version>[/go.mod] <hash>`
/// Lines with `/go.mod` suffix on the version are skipped to avoid duplicates.
///
/// Go 1.17+ with lazy module loading can record multiple versions of the same
/// module in go.sum (direct + transitive dependencies at different versions).
/// We collect all versions so the caller can choose the right one.
pub fn parse_go_sum(content: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Split: module version hash
        let mut parts = trimmed.splitn(3, ' ');
        let module = match parts.next() {
            Some(m) if !m.is_empty() => m,
            _ => continue,
        };
        let version = match parts.next() {
            Some(v) => v,
            None => continue,
        };

        // Skip /go.mod entries (duplicate of the module entry)
        if version.ends_with("/go.mod") {
            continue;
        }

        map.entry_ref(module).or_default().push(version.to_string());
    }

    map
}

/// Find the go.sum file co-located with a go.mod path.
///
/// Unlike Cargo workspaces (where Cargo.lock lives at the workspace root above member
/// Cargo.toml files), Go modules always place go.sum in the same directory as go.mod.
/// Therefore we only check the immediate directory — no upward traversal needed.
///
/// Uses async I/O to avoid blocking the Tokio executor.
pub async fn find_go_sum(go_mod_path: &Path) -> Option<PathBuf> {
    let candidate = go_mod_path.parent()?.join("go.sum");
    if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_go_sum() {
        let content = "\
github.com/pkg/errors v0.9.1 h1:FEBLx1zS214owpjy7qsBeixbURkuhQAwrK5UwLGTwt4=
github.com/pkg/errors v0.9.1/go.mod h1:bwawxfHBFNV+L2hUp1rHADufV3IMtnDRdf1r5NINEl0=
golang.org/x/text v0.14.0 h1:ScX5w1eTa3QqT8oi6+ziP7dTV1S2+ALU0bI+0zXKWiQ=
golang.org/x/text v0.14.0/go.mod h1:18ZOQIKpY8NJVqYksKHtTdi31H5itFRjB5/qKTNYzSU=
";
        let map = parse_go_sum(content);
        assert_eq!(map.get("github.com/pkg/errors").unwrap(), &["v0.9.1"]);
        assert_eq!(map.get("golang.org/x/text").unwrap(), &["v0.14.0"]);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_skips_go_mod_entries() {
        let content = "\
github.com/foo/bar v1.2.3/go.mod h1:abc=
";
        let map = parse_go_sum(content);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_empty_content() {
        let map = parse_go_sum("");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_blank_lines() {
        let content =
            "\ngithub.com/pkg/errors v0.9.1 h1:abc=\n\ngolang.org/x/text v0.14.0 h1:def=\n\n";
        let map = parse_go_sum(content);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("github.com/pkg/errors").unwrap(), &["v0.9.1"]);
        assert_eq!(map.get("golang.org/x/text").unwrap(), &["v0.14.0"]);
    }

    #[test]
    fn test_parse_malformed_lines() {
        let content = "\
onlymodule
github.com/valid/module v1.0.0 h1:abc=
";
        let map = parse_go_sum(content);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("github.com/valid/module").unwrap(), &["v1.0.0"]);
    }

    #[test]
    fn test_parse_single_module() {
        let content = "github.com/stretchr/testify v1.8.4 h1:xyz=\n";
        let map = parse_go_sum(content);
        assert_eq!(map.get("github.com/stretchr/testify").unwrap(), &["v1.8.4"]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_duplicate_module_collects_all_versions() {
        let content = "\
github.com/foo/bar v1.0.0 h1:abc=
github.com/foo/bar v1.1.0 h1:def=
";
        let map = parse_go_sum(content);
        assert_eq!(
            map.get("github.com/foo/bar").unwrap(),
            &["v1.0.0", "v1.1.0"]
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_gopkg_in_module() {
        let content = "\
gopkg.in/yaml.v3 v3.0.1 h1:abc=
gopkg.in/yaml.v3 v3.0.1/go.mod h1:def=
";
        let map = parse_go_sum(content);
        assert_eq!(map.get("gopkg.in/yaml.v3").unwrap(), &["v3.0.1"]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_go_uber_org() {
        let content = "\
go.uber.org/zap v1.27.0 h1:abc=
go.uber.org/zap v1.27.0/go.mod h1:def=
";
        let map = parse_go_sum(content);
        assert_eq!(map.get("go.uber.org/zap").unwrap(), &["v1.27.0"]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_prerelease_version() {
        let content = "\
golang.org/x/sys v0.0.0-20230101000000-abcdef012345 h1:abc=
golang.org/x/sys v0.0.0-20230101000000-abcdef012345/go.mod h1:def=
";
        let map = parse_go_sum(content);
        assert_eq!(
            map.get("golang.org/x/sys").unwrap(),
            &["v0.0.0-20230101000000-abcdef012345"]
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_parse_module_only_go_mod_line() {
        // Module with ONLY /go.mod entry should NOT appear in the map
        let content = "\
github.com/only/gomod v1.0.0/go.mod h1:abc=
";
        let map = parse_go_sum(content);
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_no_hash() {
        // go.sum requires 3 fields (module version hash), but the parser is lenient:
        // lines with only module and version are accepted rather than rejected,
        // since the hash is not used for version resolution.
        let content = "\
github.com/foo/bar v1.0.0
";
        let map = parse_go_sum(content);
        assert_eq!(map.get("github.com/foo/bar").unwrap(), &["v1.0.0"]);
    }
}
