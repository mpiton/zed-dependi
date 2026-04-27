//! Generic lockfile resolution trait + dispatch helper.
//!
//! Abstracts the per-ecosystem lockfile lookup/parse logic so that
//! [`crate::backend::ProcessingContext::process_document`] can resolve
//! versions through a single code path regardless of the manifest format.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::file_types::FileType;
use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::LockfileGraph;

#[async_trait]
pub trait LockfileResolver: Send + Sync {
    /// Locate the lockfile relative to the manifest path.
    /// Returns `None` when no lockfile exists for this ecosystem.
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf>;

    /// Parse lockfile contents into a `LockfileGraph`.
    /// On parse failure, returns an empty graph (silent — matches existing parser behavior).
    fn parse_graph(&self, lock_content: &str) -> LockfileGraph;

    /// Normalize a package name for version-map lookup.
    /// Default: identity. Override for PEP 503 (Python), lowercase (Ruby/NuGet/Composer).
    fn normalize_name(&self, name: &str) -> String {
        name.to_string()
    }

    /// Resolve the version for a single dependency from a parsed graph.
    /// Default: first-wins lookup by normalized name.
    /// Override for ecosystems with multi-version semantics (e.g., Go).
    fn resolve_version(&self, dep: &Dependency, graph: &LockfileGraph) -> Option<String> {
        let normalized = self.normalize_name(&dep.name);
        graph
            .packages
            .iter()
            .find(|p| p.name == normalized)
            .map(|p| p.version.clone())
    }
}

/// Pick the resolver matching `file_type`.
/// For Npm/Python the on-disk sub-format is probed eagerly so the resolver
/// caches the lockfile path + sub-format variant.
/// Returns `None` for `FileType::Maven` (unsupported).
pub async fn select_resolver(
    file_type: FileType,
    manifest_path: &Path,
    manifest_content: &str,
) -> Option<Box<dyn LockfileResolver>> {
    let _ = manifest_path;
    match file_type {
        FileType::Cargo => {
            let root_package =
                crate::parsers::cargo::cargo_root_package_name(manifest_content);
            Some(Box::new(crate::parsers::cargo_lock::CargoResolver {
                root_package,
            }))
        }
        FileType::Maven => None,
        _ => None,
    }
}

/// Run the resolver against `dependencies`, mutating `resolved_version` in place.
/// Returns the parsed `Arc<LockfileGraph>` for downstream consumers (vuln attribution).
pub async fn resolve_versions_from_lockfile(
    dependencies: &mut [Dependency],
    resolver: Box<dyn LockfileResolver>,
    manifest_path: &Path,
) -> Option<Arc<LockfileGraph>> {
    let lock_path = resolver.find_lockfile(manifest_path).await?;
    let lock_content = match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                "Could not read lockfile at {}: {}",
                lock_path.display(),
                e
            );
            return None;
        }
    };
    let graph = resolver.parse_graph(&lock_content);
    for dep in dependencies.iter_mut() {
        if let Some(v) = resolver.resolve_version(dep, &graph) {
            dep.resolved_version = Some(v);
        }
    }
    tracing::debug!(
        "Resolved {} versions from {}",
        dependencies
            .iter()
            .filter(|d| d.resolved_version.is_some())
            .count(),
        lock_path.display()
    );
    Some(Arc::new(graph))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn select_resolver_returns_cargo_resolver_for_cargo_filetype() {
        let path = Path::new("/tmp/Cargo.toml");
        let manifest = r#"[package]
name = "demo"
version = "0.0.1"
"#;
        let resolver = select_resolver(FileType::Cargo, path, manifest).await;
        assert!(resolver.is_some(), "Cargo should yield a resolver");
    }

    #[tokio::test]
    async fn select_resolver_returns_none_for_maven() {
        let path = Path::new("/tmp/pom.xml");
        let result = select_resolver(FileType::Maven, path, "").await;
        assert!(result.is_none(), "Maven should not produce a resolver");
    }

    struct StubResolver {
        lock_path: Option<PathBuf>,
        graph: LockfileGraph,
    }

    #[async_trait]
    impl LockfileResolver for StubResolver {
        async fn find_lockfile(&self, _manifest_path: &Path) -> Option<PathBuf> {
            self.lock_path.clone()
        }
        fn parse_graph(&self, _content: &str) -> LockfileGraph {
            LockfileGraph {
                packages: self.graph.packages.clone(),
            }
        }
    }

    fn test_dep(name: &str, version: &str) -> Dependency {
        Dependency {
            name: name.to_string(),
            version: version.to_string(),
            name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
            version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
            dev: false,
            optional: false,
            registry: None,
            resolved_version: None,
        }
    }

    fn test_pkg(name: &str, version: &str) -> crate::parsers::lockfile_graph::LockfilePackage {
        crate::parsers::lockfile_graph::LockfilePackage {
            name: name.to_string(),
            version: version.to_string(),
            dependencies: Vec::new(),
            is_root: false,
        }
    }

    #[tokio::test]
    async fn helper_returns_none_when_resolver_finds_no_lockfile() {
        let resolver: Box<dyn LockfileResolver> = Box::new(StubResolver {
            lock_path: None,
            graph: LockfileGraph { packages: vec![] },
        });
        let mut deps = vec![test_dep("serde", "1.0.0")];
        let result = resolve_versions_from_lockfile(&mut deps, resolver, Path::new("/tmp/Cargo.toml")).await;
        assert!(result.is_none());
        assert_eq!(deps[0].resolved_version, None);
    }

    #[tokio::test]
    async fn helper_resolves_versions_via_resolver() {
        use std::io::Write;
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_path = tmp.path().join("Cargo.lock");
        let mut file = std::fs::File::create(&lock_path).expect("create lockfile");
        writeln!(file, "# stub lockfile content").expect("write lockfile");

        let resolver: Box<dyn LockfileResolver> = Box::new(StubResolver {
            lock_path: Some(lock_path.clone()),
            graph: LockfileGraph {
                packages: vec![test_pkg("serde", "1.0.230"), test_pkg("tokio", "1.50.0")],
            },
        });
        let mut deps = vec![
            test_dep("serde", "1.0"),
            test_dep("tokio", "1.0"),
            test_dep("absent", "0"),
        ];
        let manifest_path = tmp.path().join("Cargo.toml");
        let arc = resolve_versions_from_lockfile(&mut deps, resolver, &manifest_path)
            .await
            .expect("expected Some(graph)");
        assert_eq!(arc.packages.len(), 2);
        assert_eq!(deps[0].resolved_version, Some("1.0.230".to_string()));
        assert_eq!(deps[1].resolved_version, Some("1.50.0".to_string()));
        assert_eq!(deps[2].resolved_version, None);
    }
}
