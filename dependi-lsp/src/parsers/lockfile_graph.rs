//! Shared graph representation for lockfile contents.

use hashbrown::HashSet;

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
    /// DFS from `root_name`; returns unique transitive packages (excluding `root_name` itself).
    /// Returns empty vec if root is unknown. Cycles are handled via a visited set.
    pub fn transitive_deps_of(&self, root_name: &str) -> Vec<&LockfilePackage> {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = Vec::new();
        let mut out: Vec<&LockfilePackage> = Vec::new();

        if let Some(root) = self.find(root_name) {
            visited.insert(&root.name);
            for dep in &root.dependencies {
                stack.push(dep.as_str());
            }
        } else {
            return out;
        }

        while let Some(name) = stack.pop() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(pkg) = self.find(name) {
                out.push(pkg);
                for dep in &pkg.dependencies {
                    stack.push(dep.as_str());
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

    fn find(&self, name: &str) -> Option<&LockfilePackage> {
        self.packages.iter().find(|p| p.name == name)
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
}
