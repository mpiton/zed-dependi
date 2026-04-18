//! Shared graph representation for lockfile contents.

use hashbrown::HashSet;
use tokio::io::AsyncReadExt;

/// Read a lockfile with a 50 MiB size cap to prevent OOM on hostile inputs.
/// The cap is enforced DURING the read, not before, to avoid TOCTOU races.
pub async fn read_lockfile_capped(path: &std::path::Path) -> std::io::Result<String> {
    const MAX_LOCKFILE_BYTES: u64 = 50 * 1024 * 1024;
    let file = tokio::fs::File::open(path).await?;
    // `take` yields at most MAX+1 bytes; if the source is longer, the extra byte
    // signals the overflow and we reject.
    let mut buf = Vec::with_capacity(4096);
    let mut reader = file.take(MAX_LOCKFILE_BYTES + 1);
    reader.read_to_end(&mut buf).await?;
    if buf.len() as u64 > MAX_LOCKFILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "lockfile exceeds {} MiB cap",
                MAX_LOCKFILE_BYTES / (1024 * 1024)
            ),
        ));
    }
    String::from_utf8(buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.utf8_error()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfilePackage {
    pub name: String,
    pub version: String,
    /// Names of packages directly required by this one (no version info).
    pub dependencies: Vec<String>,
    /// True when this package is declared in the manifest (a direct dependency).
    pub is_root: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LockfileGraph {
    pub packages: Vec<LockfilePackage>,
}

impl LockfileGraph {
    /// Returns all packages sharing the given name (0 or more). Some lockfiles
    /// pin multiple versions of the same crate/package — e.g. Cargo's
    /// transitive resolution.
    pub fn find_all(&self, name: &str) -> Vec<&LockfilePackage> {
        self.packages.iter().filter(|p| p.name == name).collect()
    }

    /// DFS from `root_name`. Returns unique transitive packages (by identity
    /// `(name, version)`), excluding roots themselves. Cycle-safe.
    ///
    /// Dependency strings may be `"name"` or `"name version"` (Cargo multi-version format).
    /// The version suffix is stripped when resolving graph edges so that both forms work.
    ///
    /// When a lockfile contains multiple versions of the same package, ALL of them
    /// are visited (package identity is `(name, version)`, not just `name`).
    pub fn transitive_deps_of(&self, root_name: &str) -> Vec<&LockfilePackage> {
        let mut visited: HashSet<(&str, &str)> = HashSet::new();
        let mut stack: Vec<&str> = Vec::new();
        let mut out: Vec<&LockfilePackage> = Vec::new();

        // Seed with every package matching root_name (multiple versions possible).
        for root in self.find_all(root_name) {
            visited.insert((&root.name, &root.version));
            for dep in &root.dependencies {
                // dep can be "name" or "name version" — use the name only for graph walk.
                let name = dep.split_whitespace().next().unwrap_or(dep.as_str());
                stack.push(name);
            }
        }

        if visited.is_empty() {
            return out;
        }

        while let Some(name) = stack.pop() {
            for pkg in self.find_all(name) {
                let key = (pkg.name.as_str(), pkg.version.as_str());
                if !visited.insert(key) {
                    continue;
                }
                out.push(pkg);
                for dep in &pkg.dependencies {
                    let n = dep.split_whitespace().next().unwrap_or(dep.as_str());
                    stack.push(n);
                }
            }
        }

        out
    }

    /// Packages that are not declared in the manifest (pure transitives).
    pub fn transitives_only(&self, manifest_deps: &[String]) -> Vec<&LockfilePackage> {
        let set: HashSet<&str> = manifest_deps.iter().map(String::as_str).collect();
        self.packages
            .iter()
            .filter(|p| !set.contains(p.name.as_str()))
            .collect()
    }

    /// Build an inverse index: for each transitive package name, the set of direct
    /// dependency names (from `manifest_deps`) that reach it via `transitive_deps_of`.
    ///
    /// Returns a `HashMap<String, Vec<String>>`. When a transitive is not reachable from
    /// any direct dep, it has no entry. Duplicate parent entries (same direct reaching
    /// the same transitive via multiple paths) are deduplicated.
    pub fn reverse_index(
        &self,
        manifest_deps: &[String],
    ) -> hashbrown::HashMap<String, Vec<String>> {
        let mut inverse: hashbrown::HashMap<String, Vec<String>> = hashbrown::HashMap::new();
        for direct in manifest_deps {
            for pkg in self.transitive_deps_of(direct) {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "`pkg.name` is &str; `entry_ref` would still allocate on insert for Vec<String>"
                )]
                inverse
                    .entry(pkg.name.clone())
                    .or_default()
                    .push(direct.clone());
            }
        }
        // Dedup parents (same direct can reach a transitive via multiple paths)
        for v in inverse.values_mut() {
            v.sort();
            v.dedup();
        }
        inverse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, deps: &[&str], is_root: bool) -> LockfilePackage {
        LockfilePackage {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            is_root,
        }
    }

    #[test]
    fn test_transitive_deps_of_single_level() {
        let graph = LockfileGraph {
            packages: vec![
                pkg("react", &["react-dom"], true),
                pkg("react-dom", &[], false),
            ],
        };
        let names: Vec<&str> = graph
            .transitive_deps_of("react")
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["react-dom"]);
    }

    #[test]
    fn test_transitive_deps_of_multi_level() {
        let graph = LockfileGraph {
            packages: vec![
                pkg("react", &["react-dom"], true),
                pkg("react-dom", &["scheduler"], false),
                pkg("scheduler", &[], false),
            ],
        };
        let names: Vec<String> = graph
            .transitive_deps_of("react")
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert!(names.contains(&"react-dom".to_string()));
        assert!(names.contains(&"scheduler".to_string()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn test_transitive_deps_of_cyclic() {
        let graph = LockfileGraph {
            packages: vec![pkg("a", &["b"], true), pkg("b", &["a"], false)],
        };
        let names: Vec<&str> = graph
            .transitive_deps_of("a")
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["b"]);
    }

    #[test]
    fn test_transitive_deps_of_unknown_root() {
        let graph = LockfileGraph { packages: vec![] };
        assert!(graph.transitive_deps_of("nope").is_empty());
    }

    #[test]
    fn test_reverse_index_attributes_transitive_to_direct() {
        let graph = LockfileGraph {
            packages: vec![
                pkg("react", &["scheduler"], true),
                pkg("vue", &["scheduler"], true),
                pkg("scheduler", &[], false),
            ],
        };
        let idx = graph.reverse_index(&["react".to_string(), "vue".to_string()]);
        let parents = idx.get("scheduler").unwrap();
        assert!(parents.contains(&"react".to_string()));
        assert!(parents.contains(&"vue".to_string()));
    }

    #[test]
    fn test_transitives_only_excludes_manifest_deps() {
        let graph = LockfileGraph {
            packages: vec![
                pkg("react", &["react-dom"], true),
                pkg("react-dom", &[], false),
                pkg("scheduler", &[], false),
            ],
        };
        let manifest: Vec<String> = vec!["react".to_string()];
        let names: Vec<String> = graph
            .transitives_only(&manifest)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        assert!(!names.contains(&"react".to_string()));
        assert!(names.contains(&"react-dom".to_string()));
        assert!(names.contains(&"scheduler".to_string()));
    }

    #[tokio::test]
    async fn test_read_lockfile_capped_rejects_oversized() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Write 51 MiB (just over the cap)
        let size = 51 * 1024 * 1024;
        let data = vec![b'a'; size];
        tmp.as_file().write_all(&data).unwrap();
        let err = read_lockfile_capped(tmp.path()).await.err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("MiB cap"));
    }

    #[tokio::test]
    async fn test_read_lockfile_capped_accepts_small_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "hello").unwrap();
        let out = read_lockfile_capped(tmp.path()).await.unwrap();
        assert_eq!(out, "hello");
    }

    #[test]
    fn test_transitive_deps_of_multi_version_visits_all() {
        // Cargo commonly pins multiple versions of the same crate. Ensure DFS
        // visits ALL same-named packages (keyed by (name, version)) rather than
        // stopping at the first match.
        let graph = LockfileGraph {
            packages: vec![
                LockfilePackage {
                    name: "root".into(),
                    version: "0.1.0".into(),
                    dependencies: vec!["hashbrown".into()],
                    is_root: true,
                },
                LockfilePackage {
                    name: "hashbrown".into(),
                    version: "0.15.5".into(),
                    dependencies: vec!["foundationdb".into()],
                    is_root: false,
                },
                LockfilePackage {
                    name: "hashbrown".into(),
                    version: "0.16.1".into(),
                    dependencies: vec!["allocator-api2".into()],
                    is_root: false,
                },
                LockfilePackage {
                    name: "foundationdb".into(),
                    version: "1.0.0".into(),
                    dependencies: vec![],
                    is_root: false,
                },
                LockfilePackage {
                    name: "allocator-api2".into(),
                    version: "0.2.0".into(),
                    dependencies: vec![],
                    is_root: false,
                },
            ],
        };

        let transitives = graph.transitive_deps_of("root");
        let names: Vec<String> = transitives
            .iter()
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect();

        // BOTH hashbrown versions must be visited
        assert!(names.iter().any(|s| s == "hashbrown@0.15.5"));
        assert!(names.iter().any(|s| s == "hashbrown@0.16.1"));

        // AND their respective children (attribution across both)
        assert!(names.iter().any(|s| s == "foundationdb@1.0.0"));
        assert!(names.iter().any(|s| s == "allocator-api2@0.2.0"));
    }

    #[test]
    fn test_find_all_returns_all_matches() {
        let graph = LockfileGraph {
            packages: vec![
                LockfilePackage {
                    name: "a".into(),
                    version: "1.0".into(),
                    dependencies: vec![],
                    is_root: false,
                },
                LockfilePackage {
                    name: "a".into(),
                    version: "2.0".into(),
                    dependencies: vec![],
                    is_root: false,
                },
                LockfilePackage {
                    name: "b".into(),
                    version: "1.0".into(),
                    dependencies: vec![],
                    is_root: false,
                },
            ],
        };
        assert_eq!(graph.find_all("a").len(), 2);
        assert_eq!(graph.find_all("b").len(), 1);
        assert_eq!(graph.find_all("c").len(), 0);
    }
}
