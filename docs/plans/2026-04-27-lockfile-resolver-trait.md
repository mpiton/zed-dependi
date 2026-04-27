# LockfileResolver Trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace 8 duplicated lockfile resolution blocks in `backend.rs::process_document` (lines 162-605, ~440 lines) with a `LockfileResolver` trait + 8 ecosystem impls + a generic helper.

**Architecture:** New module `parsers/lockfile_resolver.rs` defines the trait, `select_resolver()`, and `resolve_versions_from_lockfile()`. Each ecosystem provides a `XResolver` struct in its existing parser module and `impl LockfileResolver`. Dispatch uses `Box<dyn LockfileResolver>` via `async_trait`, mirroring the existing `AdvisoryReadCache` pattern.

**Tech Stack:** Rust, `async_trait`, `tokio`, existing `tower_lsp` LSP framework, `cargo test` for verification.

**Spec:** [`docs/specs/2026-04-27-lockfile-resolver-trait-design.md`](../specs/2026-04-27-lockfile-resolver-trait-design.md)

**Critical refinement vs spec:** Go's version resolution is non-trivial (prefer exact match, fallback to sole-candidate). The trait gains a `resolve_version(&self, dep, graph)` method with a default impl (first-wins by normalized name); `GoResolver` overrides it. This is a clarification, not a redesign.

**TDD discipline:** Every task follows Red → Green → Refactor. No production code without a failing test first.

**Test commands:**
- Module-scoped: `cargo test --lib --package dependi-lsp parsers::lockfile_resolver`
- Per-resolver: `cargo test --lib --package dependi-lsp parsers::cargo_lock::tests::resolver_`
- Integration: `cargo test --test lockfile_resolver_integration --package dependi-lsp`
- Full sweep: `cargo test --lib --package dependi-lsp && cargo test --tests --package dependi-lsp`
- Lints: `cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `dependi-lsp/src/parsers/lockfile_resolver.rs` | Trait, `select_resolver`, `resolve_versions_from_lockfile`, contract tests |
| Modify | `dependi-lsp/src/parsers/mod.rs` | Add `pub mod lockfile_resolver;` |
| Modify | `dependi-lsp/src/parsers/cargo.rs` | Move `cargo_root_package_name` here as `pub fn` (used by both backend and resolver) |
| Modify | `dependi-lsp/src/parsers/cargo_lock.rs` | Add `pub struct CargoResolver { root_package: Option<String> }` + `impl LockfileResolver` + tests |
| Modify | `dependi-lsp/src/parsers/npm_lock.rs` | Add `pub struct NpmResolver { lock_path: PathBuf, sub: NpmLockfileType }` + impl + tests |
| Modify | `dependi-lsp/src/parsers/python_lock.rs` | Add `pub struct PythonResolver { lock_path: PathBuf, sub: PythonLockfileType }` + impl + tests |
| Modify | `dependi-lsp/src/parsers/go_sum.rs` | Add `pub struct GoResolver` + impl (overrides `resolve_version`) + tests |
| Modify | `dependi-lsp/src/parsers/composer_lock.rs` | Add `pub struct PhpResolver` + impl + tests |
| Modify | `dependi-lsp/src/parsers/pubspec_lock.rs` | Add `pub struct DartResolver` + impl + tests |
| Modify | `dependi-lsp/src/parsers/packages_lock_json.rs` | Add `pub struct CsharpResolver` + impl + tests |
| Modify | `dependi-lsp/src/parsers/gemfile_lock.rs` | Add `pub struct RubyResolver` + impl + tests |
| Modify | `dependi-lsp/src/backend.rs` | Replace lines 162-605 with single helper call; remove private `cargo_root_package_name` (now in `parsers/cargo.rs`) |
| Modify | `dependi-lsp/src/main.rs` | Update its private `cargo_root_package_name` to call `parsers::cargo::cargo_root_package_name` (or keep duplicate — main.rs is a bin, not the LSP server; verify usage) |
| Create | `dependi-lsp/tests/lockfile_resolver_integration.rs` | End-to-end integration test, 1 sub-test per ecosystem |
| Modify | `CHANGELOG.md` | Add `Changed` entry under `[Unreleased]` |

---

## Phase 0: Setup

### Task 0.1: Move `cargo_root_package_name` to `parsers/cargo.rs`

**Files:**
- Modify: `dependi-lsp/src/parsers/cargo.rs` (add at top after existing imports)
- Modify: `dependi-lsp/src/backend.rs:78-85` (delete) and line 173 (update call site)
- Modify: `dependi-lsp/src/main.rs:247-252` (delete) and line 369 (update call site)

- [ ] **Step 1: Write failing test in `parsers/cargo.rs`**

Add to existing `#[cfg(test)] mod tests` block at the bottom of `parsers/cargo.rs`:

```rust
#[test]
fn test_cargo_root_package_name_returns_package_name() {
    let manifest = r#"
[package]
name = "my-crate"
version = "0.1.0"

[dependencies]
serde = "1.0"
"#;
    assert_eq!(
        cargo_root_package_name(manifest),
        Some("my-crate".to_string())
    );
}

#[test]
fn test_cargo_root_package_name_returns_none_for_workspace_only() {
    let manifest = r#"
[workspace]
members = ["crate-a"]
"#;
    assert_eq!(cargo_root_package_name(manifest), None);
}

#[test]
fn test_cargo_root_package_name_returns_none_for_invalid_toml() {
    assert_eq!(cargo_root_package_name("not [valid toml ="), None);
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::cargo::tests::test_cargo_root_package_name -- --nocapture`
Expected: FAIL — `cannot find function cargo_root_package_name`

- [ ] **Step 3: Add `cargo_root_package_name` as `pub fn` in `parsers/cargo.rs`**

Add near the top of the module body (after existing imports, before the `Parser` impl):

```rust
/// Extract the `[package].name` field from a Cargo.toml manifest.
/// Used to pass the root package name to `parse_cargo_lock` for multi-version disambiguation.
pub fn cargo_root_package_name(manifest_content: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(manifest_content).ok()?;
    value
        .get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}
```

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test --package dependi-lsp --lib parsers::cargo::tests::test_cargo_root_package_name`
Expected: 3 passed.

- [ ] **Step 5: Update `backend.rs` to use the new public function**

In `dependi-lsp/src/backend.rs`:

1. Delete lines 76-85 (the private `cargo_root_package_name` function and its doc comment).
2. At line 173 (the call site `let root_name = cargo_root_package_name(content);`), replace with:
   ```rust
   let root_name = crate::parsers::cargo::cargo_root_package_name(content);
   ```

- [ ] **Step 6: Update `main.rs` similarly**

In `dependi-lsp/src/main.rs`:
1. Delete the duplicate `fn cargo_root_package_name` near line 247 (and surrounding doc comment if any).
2. At line 369, replace `cargo_root_package_name(&content)` with `crate::parsers::cargo::cargo_root_package_name(&content)`.

- [ ] **Step 7: Run full lib test suite**

Run: `cargo test --package dependi-lsp --lib`
Expected: All previous tests still pass; 3 new tests pass.

- [ ] **Step 8: Commit**

```bash
git add dependi-lsp/src/parsers/cargo.rs dependi-lsp/src/backend.rs dependi-lsp/src/main.rs
git commit -m "refactor(parsers): expose cargo_root_package_name as pub fn

Move from backend.rs/main.rs (private duplicates) to parsers/cargo.rs
to share with upcoming LockfileResolver trait.

Refs #239"
```

---

## Phase 1: Trait Foundation

### Task 1.1: Create `lockfile_resolver.rs` with trait + first failing test

**Files:**
- Create: `dependi-lsp/src/parsers/lockfile_resolver.rs`
- Modify: `dependi-lsp/src/parsers/mod.rs` (add module declaration)

- [ ] **Step 1: Add module declaration**

Edit `dependi-lsp/src/parsers/mod.rs`. After the existing `pub mod ruby;` line (around line 73), add:

```rust
pub mod lockfile_resolver;
```

Keep the alphabetical order convention if observed; otherwise place at the end of the module list.

- [ ] **Step 2: Write failing trait existence test**

Create `dependi-lsp/src/parsers/lockfile_resolver.rs` with this content:

```rust
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
    use crate::parsers::lockfile_graph::LockfilePackage;

    fn dep(name: &str, version: &str) -> Dependency {
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

    fn pkg(name: &str, version: &str) -> LockfilePackage {
        LockfilePackage {
            name: name.to_string(),
            version: version.to_string(),
            dependencies: Vec::new(),
            is_root: false,
        }
    }

    #[tokio::test]
    async fn select_resolver_returns_none_for_maven() {
        let path = Path::new("/tmp/pom.xml");
        let result = select_resolver(FileType::Maven, path, "").await;
        assert!(result.is_none(), "Maven should not produce a resolver");
    }
}
```

- [ ] **Step 3: Run test to verify pass**

Run: `cargo test --package dependi-lsp --lib parsers::lockfile_resolver::tests`
Expected: 1 passed (`select_resolver_returns_none_for_maven`).

- [ ] **Step 4: Run clippy on the new module**

Run: `cargo clippy --package dependi-lsp --lib -- -D warnings`
Expected: No warnings.

- [ ] **Step 5: Commit**

```bash
git add dependi-lsp/src/parsers/mod.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): introduce LockfileResolver trait skeleton

Adds the trait, select_resolver, and resolve_versions_from_lockfile
helper. Resolvers are wired in subsequent commits.

Refs #239"
```

---

### Task 1.2: Test the generic helper with a stub resolver

**Files:**
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs` (extend test module)

- [ ] **Step 1: Write failing test for `resolve_versions_from_lockfile` against a stub**

Append to the `tests` module inside `lockfile_resolver.rs`:

```rust
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

#[tokio::test]
async fn helper_returns_none_when_resolver_finds_no_lockfile() {
    let resolver: Box<dyn LockfileResolver> = Box::new(StubResolver {
        lock_path: None,
        graph: LockfileGraph { packages: vec![] },
    });
    let mut deps = vec![dep("serde", "1.0.0")];
    let result =
        resolve_versions_from_lockfile(&mut deps, resolver, Path::new("/tmp/Cargo.toml"))
            .await;
    assert!(result.is_none());
    assert_eq!(deps[0].resolved_version, None);
}
```

- [ ] **Step 2: Run test to verify pass**

Run: `cargo test --package dependi-lsp --lib parsers::lockfile_resolver::tests::helper_returns_none_when_resolver_finds_no_lockfile`
Expected: PASS (no implementation change needed; the helper already returns `None` when `find_lockfile` does).

- [ ] **Step 3: Add a positive test for the success path with a real-on-disk lockfile**

Append to the `tests` module:

```rust
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
            packages: vec![pkg("serde", "1.0.230"), pkg("tokio", "1.50.0")],
        },
    });
    let mut deps = vec![
        dep("serde", "1.0"),
        dep("tokio", "1.0"),
        dep("absent", "0"),
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
```

- [ ] **Step 4: Verify `tempfile` is in dev-dependencies**

Run: `grep -n "tempfile" dependi-lsp/Cargo.toml`
Expected: present (used by other tests). If absent, add `tempfile = "3"` under `[dev-dependencies]`.

- [ ] **Step 5: Run new test**

Run: `cargo test --package dependi-lsp --lib parsers::lockfile_resolver::tests::helper_resolves_versions_via_resolver`
Expected: PASS.

- [ ] **Step 6: Run full lockfile_resolver suite**

Run: `cargo test --package dependi-lsp --lib parsers::lockfile_resolver`
Expected: 3 passed.

- [ ] **Step 7: Commit**

```bash
git add dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "test(parsers): cover LockfileResolver helper happy and miss paths

Refs #239"
```

---

## Phase 2: Per-Ecosystem Resolvers

Each task in this phase follows the same shape: write a failing resolver test, implement the resolver, wire into `select_resolver`, verify, commit.

The resolvers are independent and may be implemented by parallel subagents (one task per agent).

### Task 2.1: `CargoResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/cargo_lock.rs` (add struct + impl + tests)
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs` (wire `FileType::Cargo` arm)

- [ ] **Step 1: Write failing test in `cargo_lock.rs`**

Append a new test inside the existing `#[cfg(test)] mod tests` of `cargo_lock.rs`:

```rust
#[tokio::test]
async fn cargo_resolver_finds_and_parses_cargo_lock() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest_path = tmp.path().join("Cargo.toml");
    let lock_path = tmp.path().join("Cargo.lock");
    std::fs::write(&manifest_path, r#"[package]
name = "demo"
version = "0.1.0"
"#).expect("manifest");
    std::fs::write(
        &lock_path,
        r#"
[[package]]
name = "serde"
version = "1.0.230"

[[package]]
name = "tokio"
version = "1.50.0"
"#,
    )
    .expect("lockfile");
    let resolver = super::CargoResolver { root_package: Some("demo".to_string()) };
    let found = resolver.find_lockfile(&manifest_path).await;
    assert_eq!(found.as_deref(), Some(lock_path.as_path()));
    let content = std::fs::read_to_string(&lock_path).expect("read");
    let graph = resolver.parse_graph(&content);
    assert!(graph.packages.iter().any(|p| p.name == "serde" && p.version == "1.0.230"));
    assert!(graph.packages.iter().any(|p| p.name == "tokio" && p.version == "1.50.0"));
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::cargo_lock::tests::cargo_resolver_finds_and_parses_cargo_lock`
Expected: FAIL — `cannot find struct CargoResolver`.

- [ ] **Step 3: Implement `CargoResolver`**

Append to `cargo_lock.rs` (after the existing public functions, before `#[cfg(test)]`):

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::LockfileGraph;
use crate::parsers::lockfile_resolver::LockfileResolver;

/// Resolves versions from `Cargo.lock` for a Rust project.
pub struct CargoResolver {
    /// Captured at selection time from the manifest's `[package].name`.
    /// Used by `parse_cargo_lock_graph` for multi-version disambiguation.
    pub root_package: Option<String>,
}

#[async_trait]
impl LockfileResolver for CargoResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_cargo_lock(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        parse_cargo_lock_graph(lock_content)
    }

    fn resolve_version(&self, dep: &Dependency, graph: &LockfileGraph) -> Option<String> {
        // Cargo's parse_cargo_lock applies root-package filtering, but parse_cargo_lock_graph
        // already returns disambiguated entries. First-wins by name preserves existing semantics.
        graph
            .packages
            .iter()
            .find(|p| p.name == dep.name)
            .map(|p| p.version.clone())
    }
}
```

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test --package dependi-lsp --lib parsers::cargo_lock::tests::cargo_resolver_finds_and_parses_cargo_lock`
Expected: PASS.

- [ ] **Step 5: Wire `FileType::Cargo` arm in `select_resolver`**

In `dependi-lsp/src/parsers/lockfile_resolver.rs`, replace the body of `select_resolver` with:

```rust
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
```

- [ ] **Step 6: Add a `select_resolver` test for Cargo**

Append to `lockfile_resolver.rs::tests`:

```rust
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
```

- [ ] **Step 7: Run full lib suite**

Run: `cargo test --package dependi-lsp --lib`
Expected: All previously passing tests still pass; new tests pass.

- [ ] **Step 8: Commit**

```bash
git add dependi-lsp/src/parsers/cargo_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement CargoResolver

Adds CargoResolver wrapping find_cargo_lock + parse_cargo_lock_graph
behind the LockfileResolver trait, wired into select_resolver.

Refs #239"
```

---

### Task 2.2: `NpmResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/npm_lock.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Write failing test in `npm_lock.rs`**

Append to `npm_lock.rs`'s `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn npm_resolver_handles_package_lock() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("package.json");
    let lock = tmp.path().join("package-lock.json");
    std::fs::write(&manifest, r#"{"name":"demo","version":"0.0.1"}"#).unwrap();
    std::fs::write(
        &lock,
        r#"{
          "name": "demo",
          "version": "0.0.1",
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "demo", "version": "0.0.1" },
            "node_modules/lodash": { "version": "4.17.21" }
          }
        }"#,
    )
    .unwrap();
    let resolver = super::NpmResolver {
        lock_path: lock.clone(),
        sub: super::NpmLockfileType::PackageLock,
    };
    assert_eq!(resolver.find_lockfile(&manifest).await.as_deref(), Some(lock.as_path()));
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    assert!(graph.packages.iter().any(|p| p.name == "lodash" && p.version == "4.17.21"));
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::npm_lock::tests::npm_resolver_handles_package_lock`
Expected: FAIL — `NpmResolver` undefined.

- [ ] **Step 3: Implement `NpmResolver`**

Append to `npm_lock.rs` before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::{LockfileGraph, LockfilePackage};
use crate::parsers::lockfile_resolver::LockfileResolver;

/// Resolves versions from npm/yarn/pnpm/bun lockfiles. Sub-format is captured at selection.
pub struct NpmResolver {
    pub lock_path: PathBuf,
    pub sub: NpmLockfileType,
}

#[async_trait]
impl LockfileResolver for NpmResolver {
    async fn find_lockfile(&self, _manifest_path: &Path) -> Option<PathBuf> {
        // Path was probed at selection; return the cached value.
        Some(self.lock_path.clone())
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        match self.sub {
            NpmLockfileType::PackageLock => parse_package_lock_graph(lock_content),
            NpmLockfileType::PnpmLock => parse_pnpm_lock_graph(lock_content),
            NpmLockfileType::YarnLock => parse_yarn_lock_graph(lock_content),
            NpmLockfileType::BunLock => {
                let lock_versions = parse_npm_lockfile(lock_content, self.sub);
                LockfileGraph {
                    packages: lock_versions
                        .into_iter()
                        .map(|(name, version)| LockfilePackage {
                            name,
                            version,
                            dependencies: Vec::new(),
                            is_root: false,
                        })
                        .collect(),
                }
            }
        }
    }
}
```

If `parse_package_lock_graph` / `parse_pnpm_lock_graph` / `parse_yarn_lock_graph` are not `pub` in `npm_lock.rs`, promote them to `pub(crate)` so the impl can call them. (They are pub per existing `npm_lock.rs` — verify by grepping; if private, add `pub` to their declaration in the same commit.)

- [ ] **Step 4: Run test**

Run: `cargo test --package dependi-lsp --lib parsers::npm_lock::tests::npm_resolver_handles_package_lock`
Expected: PASS.

- [ ] **Step 5: Wire `FileType::Npm` arm in `select_resolver`**

In `lockfile_resolver.rs::select_resolver`, replace the wildcard match with:

```rust
match file_type {
    FileType::Cargo => {
        let root_package =
            crate::parsers::cargo::cargo_root_package_name(manifest_content);
        Some(Box::new(crate::parsers::cargo_lock::CargoResolver {
            root_package,
        }))
    }
    FileType::Npm => {
        let (lock_path, sub) =
            crate::parsers::npm_lock::find_npm_lockfile(manifest_path).await?;
        Some(Box::new(crate::parsers::npm_lock::NpmResolver { lock_path, sub }))
    }
    FileType::Maven => None,
    _ => None,
}
```

- [ ] **Step 6: Run full lib suite + commit**

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/npm_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement NpmResolver

Wraps the 4 npm sub-formats (package-lock, pnpm-lock, yarn.lock, bun.lock)
behind LockfileResolver, with sub-format probed at selection time.

Refs #239"
```

---

### Task 2.3: `PythonResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/python_lock.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Write failing test in `python_lock.rs`**

Append to `python_lock.rs`'s `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn python_resolver_handles_poetry_lock_with_pep503_normalization() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("pyproject.toml");
    let lock = tmp.path().join("poetry.lock");
    std::fs::write(&manifest, "[tool.poetry]\nname='demo'\nversion='0.1.0'\n").unwrap();
    std::fs::write(
        &lock,
        r#"
[[package]]
name = "Some-Package"
version = "1.2.3"

[[package]]
name = "another_pkg"
version = "0.5.0"
"#,
    )
    .unwrap();
    let resolver = super::PythonResolver {
        lock_path: lock.clone(),
        sub: super::PythonLockfileType::PoetryLock,
    };
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    let dep = crate::parsers::Dependency {
        name: "some.package".to_string(),
        version: "*".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    // PEP 503 should match "some.package" → "some-package" against "Some-Package"
    let v = resolver.resolve_version(&dep, &graph);
    assert_eq!(v.as_deref(), Some("1.2.3"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::python_lock::tests::python_resolver_handles_poetry_lock_with_pep503_normalization`
Expected: FAIL — `PythonResolver` undefined.

- [ ] **Step 3: Implement `PythonResolver`**

Append to `python_lock.rs` before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::{LockfileGraph, LockfilePackage};
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct PythonResolver {
    pub lock_path: PathBuf,
    pub sub: PythonLockfileType,
}

#[async_trait]
impl LockfileResolver for PythonResolver {
    async fn find_lockfile(&self, _manifest_path: &Path) -> Option<PathBuf> {
        Some(self.lock_path.clone())
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        match self.sub {
            PythonLockfileType::PoetryLock => parse_poetry_lock_graph(lock_content),
            PythonLockfileType::UvLock => parse_uv_lock_graph(lock_content),
            PythonLockfileType::PipfileLock => parse_pipfile_lock_graph(lock_content),
            PythonLockfileType::PdmLock => {
                let lock_versions = parse_python_lockfile(lock_content, self.sub);
                LockfileGraph {
                    packages: lock_versions
                        .into_iter()
                        .map(|(name, version)| LockfilePackage {
                            name,
                            version,
                            dependencies: Vec::new(),
                            is_root: false,
                        })
                        .collect(),
                }
            }
        }
    }

    fn normalize_name(&self, name: &str) -> String {
        normalize_python_name(name)
    }

    fn resolve_version(&self, dep: &Dependency, graph: &LockfileGraph) -> Option<String> {
        let normalized_dep = self.normalize_name(&dep.name);
        graph
            .packages
            .iter()
            .find(|p| self.normalize_name(&p.name) == normalized_dep)
            .map(|p| p.version.clone())
    }
}
```

If `parse_*_lock_graph` are not `pub`, promote them. Verify with `grep -n "fn parse_.*_lock_graph" dependi-lsp/src/parsers/python_lock.rs`.

- [ ] **Step 4: Run test**

Run: `cargo test --package dependi-lsp --lib parsers::python_lock::tests::python_resolver_handles_poetry_lock_with_pep503_normalization`
Expected: PASS.

- [ ] **Step 5: Wire `FileType::Python` arm in `select_resolver`**

Add to the `match` in `select_resolver`:

```rust
FileType::Python => {
    let preferred = crate::parsers::python_lock::detect_python_tool(manifest_content);
    let (lock_path, sub) =
        crate::parsers::python_lock::find_python_lockfile(manifest_path, preferred).await?;
    Some(Box::new(crate::parsers::python_lock::PythonResolver { lock_path, sub }))
}
```

- [ ] **Step 6: Run full suite + commit**

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/python_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement PythonResolver with PEP 503 normalization

Wraps the 4 Python sub-formats (poetry, uv, pdm, Pipfile) behind
LockfileResolver, applying normalize_python_name on lookup.

Refs #239"
```

---

### Task 2.4: `GoResolver` (overrides `resolve_version` for multi-version semantics)

**Files:**
- Modify: `dependi-lsp/src/parsers/go_sum.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Write failing test capturing Go's exact-match-or-single-only rule**

Append to `go_sum.rs`'s `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn go_resolver_prefers_dep_version_when_present_in_candidates() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("go.mod");
    let lock = tmp.path().join("go.sum");
    std::fs::write(&manifest, "module example.com/demo\n").unwrap();
    std::fs::write(
        &lock,
        "github.com/foo/bar v1.0.0 h1:hash1=\n\
         github.com/foo/bar v1.1.0 h1:hash2=\n\
         github.com/baz/qux v0.5.0 h1:hash3=\n",
    )
    .unwrap();
    let resolver = super::GoResolver;
    assert_eq!(
        resolver.find_lockfile(&manifest).await.as_deref(),
        Some(lock.as_path())
    );
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);

    // dep.version matches one of the candidates → prefer it
    let dep_with_match = crate::parsers::Dependency {
        name: "github.com/foo/bar".to_string(),
        version: "v1.1.0".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    assert_eq!(
        resolver.resolve_version(&dep_with_match, &graph),
        Some("v1.1.0".to_string())
    );

    // single candidate → auto-select even when dep.version differs
    let dep_single = crate::parsers::Dependency {
        name: "github.com/baz/qux".to_string(),
        version: "v0.4.0".to_string(),
        ..dep_with_match.clone()
    };
    assert_eq!(
        resolver.resolve_version(&dep_single, &graph),
        Some("v0.5.0".to_string())
    );

    // ambiguous candidates without exact match → None
    let dep_ambiguous = crate::parsers::Dependency {
        name: "github.com/foo/bar".to_string(),
        version: "v0.0.1".to_string(),
        ..dep_with_match
    };
    assert_eq!(resolver.resolve_version(&dep_ambiguous, &graph), None);
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::go_sum::tests::go_resolver_prefers_dep_version_when_present_in_candidates`
Expected: FAIL — `GoResolver` undefined.

- [ ] **Step 3: Implement `GoResolver`**

Append to `go_sum.rs` before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::{LockfileGraph, LockfilePackage};
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct GoResolver;

#[async_trait]
impl LockfileResolver for GoResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_go_sum(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        let lock_versions = parse_go_sum(lock_content);
        let mut packages = Vec::new();
        for (name, versions) in &lock_versions {
            for version in versions {
                packages.push(LockfilePackage {
                    name: name.clone(),
                    version: version.clone(),
                    dependencies: Vec::new(),
                    is_root: false,
                });
            }
        }
        LockfileGraph { packages }
    }

    fn resolve_version(&self, dep: &Dependency, graph: &LockfileGraph) -> Option<String> {
        let candidates: Vec<&str> = graph
            .packages
            .iter()
            .filter(|p| p.name == dep.name)
            .map(|p| p.version.as_str())
            .collect();
        if candidates.iter().any(|v| *v == dep.version) {
            Some(dep.version.clone())
        } else if candidates.len() == 1 {
            Some(candidates[0].to_string())
        } else {
            None
        }
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --package dependi-lsp --lib parsers::go_sum::tests::go_resolver_prefers_dep_version_when_present_in_candidates`
Expected: PASS.

- [ ] **Step 5: Wire `FileType::Go` arm in `select_resolver`**

Add to the `match`:

```rust
FileType::Go => Some(Box::new(crate::parsers::go_sum::GoResolver)),
```

- [ ] **Step 6: Run full suite + commit**

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/go_sum.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement GoResolver with multi-version disambiguation

GoResolver overrides resolve_version to honor go.sum's multi-version
semantics: prefer dep.version when present, fallback to sole candidate,
otherwise leave unresolved.

Refs #239"
```

---

### Task 2.5: `PhpResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/composer_lock.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Write failing test in `composer_lock.rs`**

Append to its `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn php_resolver_normalizes_composer_names() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("composer.json");
    let lock = tmp.path().join("composer.lock");
    std::fs::write(&manifest, "{}").unwrap();
    std::fs::write(
        &lock,
        r#"{
          "packages": [
            { "name": "Vendor/Package", "version": "1.2.3" }
          ]
        }"#,
    )
    .unwrap();
    let resolver = super::PhpResolver;
    assert_eq!(
        resolver.find_lockfile(&manifest).await.as_deref(),
        Some(lock.as_path())
    );
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    let dep = crate::parsers::Dependency {
        name: "VENDOR/Package".to_string(),
        version: "*".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    assert_eq!(
        resolver.resolve_version(&dep, &graph),
        Some("1.2.3".to_string())
    );
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::composer_lock::tests::php_resolver_normalizes_composer_names`
Expected: FAIL.

- [ ] **Step 3: Implement `PhpResolver`**

Append before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::lockfile_graph::LockfileGraph;
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct PhpResolver;

#[async_trait]
impl LockfileResolver for PhpResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_composer_lock(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        parse_composer_lock_graph(lock_content)
    }

    fn normalize_name(&self, name: &str) -> String {
        normalize_composer_name(name)
    }
}
```

- [ ] **Step 4: Test pass**

Run: `cargo test --package dependi-lsp --lib parsers::composer_lock::tests::php_resolver_normalizes_composer_names`
Expected: PASS.

- [ ] **Step 5: Wire `FileType::Php`**

Add: `FileType::Php => Some(Box::new(crate::parsers::composer_lock::PhpResolver)),`

- [ ] **Step 6: Commit**

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/composer_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement PhpResolver

Refs #239"
```

---

### Task 2.6: `DartResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/pubspec_lock.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Failing test**

Append to `pubspec_lock.rs::tests`:

```rust
#[tokio::test]
async fn dart_resolver_resolves_simple_pubspec_lock() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("pubspec.yaml");
    let lock = tmp.path().join("pubspec.lock");
    std::fs::write(&manifest, "name: demo\nversion: 0.1.0\n").unwrap();
    std::fs::write(
        &lock,
        r#"packages:
  http:
    dependency: "direct main"
    description:
      name: http
      url: "https://pub.dev"
    source: hosted
    version: "1.2.0"
"#,
    )
    .unwrap();
    let resolver = super::DartResolver;
    assert_eq!(
        resolver.find_lockfile(&manifest).await.as_deref(),
        Some(lock.as_path())
    );
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    let dep = crate::parsers::Dependency {
        name: "http".to_string(),
        version: "^1.0.0".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    assert_eq!(resolver.resolve_version(&dep, &graph), Some("1.2.0".to_string()));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --package dependi-lsp --lib parsers::pubspec_lock::tests::dart_resolver_resolves_simple_pubspec_lock`
Expected: FAIL.

- [ ] **Step 3: Implement**

Append before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::lockfile_graph::{LockfileGraph, LockfilePackage};
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct DartResolver;

#[async_trait]
impl LockfileResolver for DartResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_pubspec_lock(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        let lock_versions = parse_pubspec_lock(lock_content);
        LockfileGraph {
            packages: lock_versions
                .into_iter()
                .map(|(name, version)| LockfilePackage {
                    name,
                    version,
                    dependencies: Vec::new(),
                    is_root: false,
                })
                .collect(),
        }
    }
}
```

- [ ] **Step 4: Pass + wire + commit**

```bash
cargo test --package dependi-lsp --lib parsers::pubspec_lock
```

Add to `select_resolver`:
```rust
FileType::Dart => Some(Box::new(crate::parsers::pubspec_lock::DartResolver)),
```

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/pubspec_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement DartResolver

Refs #239"
```

---

### Task 2.7: `CsharpResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/packages_lock_json.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Failing test**

Append to `packages_lock_json.rs::tests`:

```rust
#[tokio::test]
async fn csharp_resolver_normalizes_nuget_names() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("Demo.csproj");
    let lock = tmp.path().join("packages.lock.json");
    std::fs::write(&manifest, "<Project></Project>").unwrap();
    std::fs::write(
        &lock,
        r#"{
          "version": 1,
          "dependencies": {
            "net8.0": {
              "Newtonsoft.Json": { "type": "Direct", "resolved": "13.0.3" }
            }
          }
        }"#,
    )
    .unwrap();
    let resolver = super::CsharpResolver;
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    let dep = crate::parsers::Dependency {
        name: "newtonsoft.json".to_string(),
        version: "*".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    assert_eq!(
        resolver.resolve_version(&dep, &graph),
        Some("13.0.3".to_string())
    );
}
```

- [ ] **Step 2: Failure**

Run: `cargo test --package dependi-lsp --lib parsers::packages_lock_json::tests::csharp_resolver_normalizes_nuget_names`
Expected: FAIL.

- [ ] **Step 3: Implement**

Append before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::lockfile_graph::{LockfileGraph, LockfilePackage};
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct CsharpResolver;

#[async_trait]
impl LockfileResolver for CsharpResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_packages_lock(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        let lock_versions = parse_packages_lock(lock_content);
        LockfileGraph {
            packages: lock_versions
                .into_iter()
                .map(|(name, version)| LockfilePackage {
                    name,
                    version,
                    dependencies: Vec::new(),
                    is_root: false,
                })
                .collect(),
        }
    }

    fn normalize_name(&self, name: &str) -> String {
        normalize_nuget_name(name)
    }
}
```

- [ ] **Step 4: Pass + wire + commit**

```bash
cargo test --package dependi-lsp --lib parsers::packages_lock_json
```

In `select_resolver`:
```rust
FileType::Csharp => Some(Box::new(crate::parsers::packages_lock_json::CsharpResolver)),
```

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/packages_lock_json.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement CsharpResolver

Refs #239"
```

---

### Task 2.8: `RubyResolver`

**Files:**
- Modify: `dependi-lsp/src/parsers/gemfile_lock.rs`
- Modify: `dependi-lsp/src/parsers/lockfile_resolver.rs`

- [ ] **Step 1: Failing test**

Append to `gemfile_lock.rs::tests`:

```rust
#[tokio::test]
async fn ruby_resolver_normalizes_gem_names() {
    use crate::parsers::lockfile_resolver::LockfileResolver;
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = tmp.path().join("Gemfile");
    let lock = tmp.path().join("Gemfile.lock");
    std::fs::write(&manifest, "source 'https://rubygems.org'\ngem 'rails'\n").unwrap();
    std::fs::write(
        &lock,
        r#"GEM
  remote: https://rubygems.org/
  specs:
    rails (7.1.0)

PLATFORMS
  ruby

DEPENDENCIES
  rails

BUNDLED WITH
   2.4.0
"#,
    )
    .unwrap();
    let resolver = super::RubyResolver;
    let content = std::fs::read_to_string(&lock).unwrap();
    let graph = resolver.parse_graph(&content);
    let dep = crate::parsers::Dependency {
        name: "Rails".to_string(),
        version: "*".to_string(),
        name_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        version_span: crate::parsers::Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    };
    assert_eq!(resolver.resolve_version(&dep, &graph), Some("7.1.0".to_string()));
}
```

- [ ] **Step 2: Failure**

Run: `cargo test --package dependi-lsp --lib parsers::gemfile_lock::tests::ruby_resolver_normalizes_gem_names`
Expected: FAIL.

- [ ] **Step 3: Implement**

Append before `#[cfg(test)]`:

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::parsers::lockfile_graph::LockfileGraph;
use crate::parsers::lockfile_resolver::LockfileResolver;

pub struct RubyResolver;

#[async_trait]
impl LockfileResolver for RubyResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        find_gemfile_lock(manifest_path).await
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        parse_gemfile_lock_graph(lock_content)
    }

    fn normalize_name(&self, name: &str) -> String {
        normalize_gem_name(name)
    }
}
```

If `find_gemfile_lock` is missing in `gemfile_lock.rs`, it must be added (mirroring `find_pubspec_lock`/`find_composer_lock`). Verify with `grep -n "fn find_gemfile_lock" dependi-lsp/src/parsers/gemfile_lock.rs` before writing the impl. If absent, add a `pub async fn find_gemfile_lock(manifest_path: &Path) -> Option<PathBuf>` that returns `manifest_path.parent()?.join("Gemfile.lock")` after `tokio::fs::try_exists` confirms presence. Add a unit test for it in the same task.

- [ ] **Step 4: Pass + wire + commit**

```bash
cargo test --package dependi-lsp --lib parsers::gemfile_lock
```

In `select_resolver`:
```rust
FileType::Ruby => Some(Box::new(crate::parsers::gemfile_lock::RubyResolver)),
```

```bash
cargo test --package dependi-lsp --lib parsers
git add dependi-lsp/src/parsers/gemfile_lock.rs dependi-lsp/src/parsers/lockfile_resolver.rs
git commit -m "feat(parsers): implement RubyResolver

Refs #239"
```

---

## Phase 3: Migrate `process_document`

### Task 3.1: Replace 8 inline blocks with single helper call

**Files:**
- Modify: `dependi-lsp/src/backend.rs:162-605` (delete) and around line 162 (insert helper call)

- [ ] **Step 1: Verify all 8 resolvers green from Phase 2**

Run: `cargo test --package dependi-lsp --lib parsers`
Expected: All resolver tests pass.

- [ ] **Step 2: Re-read backend.rs lines 150-160 (context above the blocks) and 605-650 (context below)**

Use Read tool: `Read /home/matvei/projets/zed-dependi/dependi-lsp/src/backend.rs offset=150 limit=15` and `offset=605 limit=45` to confirm exact surrounding code, including the `lockfile_graph` declaration (likely an `Option<Arc<...>>` initialized to `None` above the blocks).

- [ ] **Step 3: Delete lines 162-605 and insert the helper call**

In `backend.rs`, locate the variable initialization above line 162 (likely `let mut lockfile_graph: Option<Arc<LockfileGraph>> = None;` — confirm exact name in step 2). Replace lines 162-605 with:

```rust
let lockfile_graph = if let Ok(manifest_path) = uri.to_file_path() {
    if let Some(resolver) = crate::parsers::lockfile_resolver::select_resolver(
        file_type,
        &manifest_path,
        content,
    )
    .await
    {
        crate::parsers::lockfile_resolver::resolve_versions_from_lockfile(
            &mut dependencies,
            resolver,
            &manifest_path,
        )
        .await
    } else {
        None
    }
} else {
    None
};
```

If the original code declared `let mut lockfile_graph: Option<...> = None;` on a separate line, also remove that declaration since the new expression returns the value directly. If `lockfile_graph` is mutated after this block, keep it as `let mut lockfile_graph = ...` instead of `let lockfile_graph = ...`.

- [ ] **Step 4: Build to detect type / borrow issues**

Run: `cargo build --package dependi-lsp 2>&1 | head -80`
Expected: Compiles. If borrow errors arise (e.g., `&mut dependencies` after the block), inspect downstream uses and adjust scope.

- [ ] **Step 5: Run full lib + integration suite**

```bash
cargo test --package dependi-lsp --lib
cargo test --package dependi-lsp --test integration_test
```
Expected: All previous tests pass.

- [ ] **Step 6: Commit**

```bash
git add dependi-lsp/src/backend.rs
git commit -m "refactor(backend): use LockfileResolver in process_document

Replaces 8 duplicated lockfile resolution blocks (lines 162-605, ~440 lines)
with select_resolver + resolve_versions_from_lockfile.

Refs #239"
```

---

## Phase 4: Integration Test

### Task 4.1: End-to-end test covering all 8 ecosystems

**Files:**
- Create: `dependi-lsp/tests/lockfile_resolver_integration.rs`

- [ ] **Step 1: Write the integration test**

Create `dependi-lsp/tests/lockfile_resolver_integration.rs`:

```rust
//! End-to-end coverage for `LockfileResolver` trait + dispatch helpers.
//! For each ecosystem, materialize a synthetic manifest+lockfile pair under
//! a tempdir, run the helper, and assert that `dep.resolved_version` is set.

use std::path::Path;

use dependi_lsp::file_types::FileType;
use dependi_lsp::parsers::Dependency;
use dependi_lsp::parsers::Span;
use dependi_lsp::parsers::lockfile_resolver::{
    resolve_versions_from_lockfile, select_resolver,
};

fn dep(name: &str, version: &str) -> Dependency {
    Dependency {
        name: name.to_string(),
        version: version.to_string(),
        name_span: Span { line: 0, line_start: 0, line_end: 0 },
        version_span: Span { line: 0, line_start: 0, line_end: 0 },
        dev: false,
        optional: false,
        registry: None,
        resolved_version: None,
    }
}

async fn run_resolver(
    file_type: FileType,
    manifest_path: &Path,
    manifest_content: &str,
    deps: &mut [Dependency],
) {
    let resolver = select_resolver(file_type, manifest_path, manifest_content)
        .await
        .expect("resolver should be selected");
    let _arc = resolve_versions_from_lockfile(deps, resolver, manifest_path).await;
}

#[tokio::test]
async fn cargo_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("Cargo.toml");
    std::fs::write(&manifest, r#"[package]
name = "demo"
version = "0.1.0"
[dependencies]
serde = "1"
"#).unwrap();
    std::fs::write(
        tmp.path().join("Cargo.lock"),
        r#"
[[package]]
name = "serde"
version = "1.0.230"
"#,
    )
    .unwrap();
    let mut deps = vec![dep("serde", "1")];
    run_resolver(
        FileType::Cargo,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("1.0.230".to_string()));
}

#[tokio::test]
async fn npm_end_to_end_package_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("package.json");
    std::fs::write(&manifest, r#"{"name":"demo","version":"0.0.1"}"#).unwrap();
    std::fs::write(
        tmp.path().join("package-lock.json"),
        r#"{
          "name":"demo","version":"0.0.1","lockfileVersion":3,
          "packages":{
            "":{"name":"demo","version":"0.0.1"},
            "node_modules/lodash":{"version":"4.17.21"}
          }
        }"#,
    )
    .unwrap();
    let mut deps = vec![dep("lodash", "^4.0.0")];
    run_resolver(
        FileType::Npm,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("4.17.21".to_string()));
}

#[tokio::test]
async fn python_end_to_end_poetry() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("pyproject.toml");
    std::fs::write(&manifest, "[tool.poetry]\nname='demo'\nversion='0.1.0'\n").unwrap();
    std::fs::write(
        tmp.path().join("poetry.lock"),
        r#"
[[package]]
name = "Some-Package"
version = "1.2.3"
"#,
    )
    .unwrap();
    let mut deps = vec![dep("some.package", "*")];
    run_resolver(
        FileType::Python,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("1.2.3".to_string()));
}

#[tokio::test]
async fn go_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("go.mod");
    std::fs::write(&manifest, "module example.com/demo\n").unwrap();
    std::fs::write(
        tmp.path().join("go.sum"),
        "github.com/foo/bar v1.0.0 h1:hash=\n",
    )
    .unwrap();
    let mut deps = vec![dep("github.com/foo/bar", "v1.0.0")];
    run_resolver(
        FileType::Go,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("v1.0.0".to_string()));
}

#[tokio::test]
async fn php_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("composer.json");
    std::fs::write(&manifest, "{}").unwrap();
    std::fs::write(
        tmp.path().join("composer.lock"),
        r#"{"packages":[{"name":"vendor/pkg","version":"1.0.0"}]}"#,
    )
    .unwrap();
    let mut deps = vec![dep("VENDOR/PKG", "*")];
    run_resolver(
        FileType::Php,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("1.0.0".to_string()));
}

#[tokio::test]
async fn dart_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("pubspec.yaml");
    std::fs::write(&manifest, "name: demo\nversion: 0.1.0\n").unwrap();
    std::fs::write(
        tmp.path().join("pubspec.lock"),
        r#"packages:
  http:
    dependency: "direct main"
    description:
      name: http
      url: "https://pub.dev"
    source: hosted
    version: "1.2.0"
"#,
    )
    .unwrap();
    let mut deps = vec![dep("http", "^1.0.0")];
    run_resolver(
        FileType::Dart,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("1.2.0".to_string()));
}

#[tokio::test]
async fn csharp_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("Demo.csproj");
    std::fs::write(&manifest, "<Project></Project>").unwrap();
    std::fs::write(
        tmp.path().join("packages.lock.json"),
        r#"{"version":1,"dependencies":{"net8.0":{"Newtonsoft.Json":{"type":"Direct","resolved":"13.0.3"}}}}"#,
    )
    .unwrap();
    let mut deps = vec![dep("newtonsoft.json", "*")];
    run_resolver(
        FileType::Csharp,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("13.0.3".to_string()));
}

#[tokio::test]
async fn ruby_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("Gemfile");
    std::fs::write(&manifest, "source 'https://rubygems.org'\ngem 'rails'\n").unwrap();
    std::fs::write(
        tmp.path().join("Gemfile.lock"),
        r#"GEM
  remote: https://rubygems.org/
  specs:
    rails (7.1.0)

PLATFORMS
  ruby

DEPENDENCIES
  rails

BUNDLED WITH
   2.4.0
"#,
    )
    .unwrap();
    let mut deps = vec![dep("rails", "*")];
    run_resolver(
        FileType::Ruby,
        &manifest,
        &std::fs::read_to_string(&manifest).unwrap(),
        &mut deps,
    )
    .await;
    assert_eq!(deps[0].resolved_version, Some("7.1.0".to_string()));
}

#[tokio::test]
async fn maven_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("pom.xml");
    std::fs::write(&manifest, "<project></project>").unwrap();
    let resolver = select_resolver(FileType::Maven, &manifest, "<project></project>").await;
    assert!(resolver.is_none(), "Maven not supported");
}
```

- [ ] **Step 2: Verify the binary crate exposes the modules used by the test**

If `dependi-lsp` is binary-only, integration tests cannot reach internals. Check:

```bash
ls dependi-lsp/src/lib.rs 2>&1 || echo "NO LIB"
```

If `lib.rs` does not exist, examine `Cargo.toml` for `[lib]` section. If neither exists, the integration test **must be added as `#[cfg(test)] mod tests` inside the binary**, OR a `lib.rs` re-export must be added.

If `lib.rs` exists, ensure it re-exports `pub mod parsers; pub mod file_types;` (or that they are already `pub`).

If a new `lib.rs` is needed, create a minimal one:

```rust
// dependi-lsp/src/lib.rs
pub mod auth;
pub mod backend;
pub mod cache;
pub mod config;
pub mod document;
pub mod file_types;
pub mod parsers;
pub mod providers;
pub mod registries;
pub mod reports;
pub mod utils;
pub mod vulnerabilities;
```

(Adjust to match the actual modules in `src/`.) Update `Cargo.toml` `[lib]` and `[[bin]]` accordingly. **If the existing project structure does not support a lib crate, replace the integration test path with `dependi-lsp/src/parsers/lockfile_resolver_integration_test.rs` registered via `mod` inside the trait module under `#[cfg(test)]`** (preferred fallback — fewer structural changes).

- [ ] **Step 3: Run integration test**

Run: `cargo test --package dependi-lsp --test lockfile_resolver_integration`
Expected: 9 passed (8 ecosystems + Maven None case).

- [ ] **Step 4: Commit**

```bash
git add dependi-lsp/tests/lockfile_resolver_integration.rs dependi-lsp/Cargo.toml dependi-lsp/src/lib.rs 2>/dev/null
git commit -m "test(integration): cover all 8 LockfileResolver ecosystems end-to-end

Refs #239"
```

If `Cargo.toml`/`lib.rs` weren't touched, the `git add` for those will be a no-op.

---

## Phase 5: Verification & Cleanup

### Task 5.1: Lint and format

- [ ] **Step 1: Run clippy**

Run: `cargo clippy --package dependi-lsp --all-targets -- -D warnings`
Expected: No warnings.

If warnings appear, fix them inline (no `#[allow(...)]` per CLAUDE.md). Common: unused `_ = ...` lines, missing `#[must_use]`, redundant clones.

- [ ] **Step 2: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: Clean.

If not clean: `cargo fmt --all` + commit:

```bash
git add -u
git commit -m "style: cargo fmt"
```

- [ ] **Step 3: Run the full test sweep**

```bash
cargo test --package dependi-lsp --lib
cargo test --package dependi-lsp --tests
```
Expected: Everything green.

- [ ] **Step 4: Verify line count reduction**

Run: `git diff main..HEAD -- dependi-lsp/src/backend.rs | grep -c "^-"`
Expected: ≥400 deletions in `backend.rs`.

- [ ] **Step 5: No `unwrap`/`expect` in main code**

Run: `grep -nE "\\.(unwrap|expect)\\(" dependi-lsp/src/parsers/lockfile_resolver.rs dependi-lsp/src/parsers/cargo_lock.rs dependi-lsp/src/parsers/npm_lock.rs dependi-lsp/src/parsers/python_lock.rs dependi-lsp/src/parsers/go_sum.rs dependi-lsp/src/parsers/composer_lock.rs dependi-lsp/src/parsers/pubspec_lock.rs dependi-lsp/src/parsers/packages_lock_json.rs dependi-lsp/src/parsers/gemfile_lock.rs | grep -v "tests::\|#\\[cfg(test)\\]\|fn test_\|let tmp = tempfile" || echo "no unwrap/expect outside tests"`

Expected: `no unwrap/expect outside tests` (test code is exempt per CLAUDE.md spirit).

### Task 5.2: Update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Read current `[Unreleased]` section**

Run: `cargo run --bin /bin/sh -- -c 'sed -n "1,40p" CHANGELOG.md'` — actually use Read tool:

Read `/home/matvei/projets/zed-dependi/CHANGELOG.md` lines 1-40 to see the format used.

- [ ] **Step 2: Add `Changed` entry under `[Unreleased]`**

In the `[Unreleased]` section, under a `### Changed` heading (create if absent), add:

```markdown
- Refactored `process_document` lockfile resolution into the new `LockfileResolver` trait, reducing ~440 duplicated lines across 8 ecosystems and easing future ecosystem additions ([#239](https://github.com/Mathieu-Piton/zed-dependi/issues/239)).
```

- [ ] **Step 3: Commit CHANGELOG**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): note LockfileResolver trait refactor"
```

---

## Phase 6: Multi-Agent Review

### Task 6.1: Dispatch parallel reviewers

After all implementation tasks are complete and tests are green, dispatch the following review agents in **parallel** (single message, multiple `Agent` invocations):

- [ ] **Step 1: Dispatch reviewers in parallel**

Use the Agent tool with `subagent_type` values:

1. `rust-reviewer` — idiomatic Rust, ownership, lifetimes, async patterns; focus on the new trait + impls.
2. `code-reviewer` — generic quality, security, maintainability; focus on the helper, selector, and migrated `process_document`.
3. `superpowers:code-reviewer` — adversarial check against the spec at `docs/specs/2026-04-27-lockfile-resolver-trait-design.md`.
4. `tdd-guide` — verify TDD discipline (no impl without prior failing test) by inspecting commit history.

Each agent must be briefed with: (a) the spec path, (b) the plan path (this file), (c) the branch name, (d) the request to focus on issue-239 changes only.

- [ ] **Step 2: Address review feedback**

Incorporate critical/blocking comments. Open follow-up commits per reviewer. Skip nits unless several reviewers raise the same point.

- [ ] **Step 3: Final test sweep + push**

```bash
cargo test --package dependi-lsp --lib
cargo test --package dependi-lsp --tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
git push -u origin feat/issue-239-lockfile-resolver-trait
```

- [ ] **Step 4: Open PR**

Use `gh pr create` with title `refactor(parsers): introduce LockfileResolver trait (#239)` and a body summarizing the spec, line reduction, and acceptance criteria status.

---

## Acceptance Checklist

- [ ] Spec coverage: every section of the spec maps to at least one task.
- [ ] No placeholders in the plan or in committed code.
- [ ] All resolver tests pass: `cargo test --package dependi-lsp --lib parsers`.
- [ ] Integration test passes: `cargo test --package dependi-lsp --test lockfile_resolver_integration`.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] No new `unwrap`/`expect` outside tests.
- [ ] No new `#[allow(...)]`.
- [ ] `process_document` reduced by ~400 lines.
- [ ] CHANGELOG `[Unreleased]` updated under `Changed`.
- [ ] All 4 review agents dispatched and feedback addressed.
