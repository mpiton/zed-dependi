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
    let _ = (manifest_path, manifest_content);
    match file_type {
        FileType::Maven => None,
        // Other variants implemented in subsequent tasks.
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
    async fn select_resolver_returns_none_for_maven() {
        let path = Path::new("/tmp/pom.xml");
        let result = select_resolver(FileType::Maven, path, "").await;
        assert!(result.is_none(), "Maven should not produce a resolver");
    }
}
