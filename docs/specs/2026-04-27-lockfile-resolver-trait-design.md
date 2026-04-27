# Lockfile Resolver Trait — Design Spec

- **Date**: 2026-04-27
- **Issue**: [#239](https://github.com/Mathieu-Piton/zed-dependi/issues/239)
- **Status**: Approved
- **Author**: Mathieu Piton

## 1. Problem

`dependi-lsp/src/backend.rs::process_document` (lines 162-605) contains 8 near-identical `if file_type == X` blocks for resolving dependency versions from lockfiles across ecosystems (Cargo, npm, Python, Go, PHP, Dart, C#, Ruby). Approximately 440 lines of duplicated structure: each block finds the lockfile, reads it via `read_lockfile_capped`, parses a graph, builds a `version_map`, mutates `dependencies`, and stores an `Arc<LockfileGraph>`.

Maintenance overhead is high: any change to logging, error semantics, or graph handling must be replicated 8 times. Adding a new ecosystem requires copy-pasting the pattern.

## 2. Goals

- Eliminate the 8 duplicated blocks via a trait abstraction.
- Reduce `process_document` by ~400 lines without changing observable behavior.
- Preserve current logging, error swallowing, and `lockfile_graph` Arc semantics.
- Keep API surface internal to the crate (no public breakage).
- Enable future ecosystem additions via a single `impl LockfileResolver`.

## 3. Non-Goals

- Re-design error handling (parsers continue swallowing parse failures silently).
- Add support for new ecosystems (Maven remains unsupported).
- Modify `LockfileGraph`, `LockfilePackage`, or `Dependency` structs.
- Refactor downstream consumers of `lockfile_graph` (transitive vuln attribution, diagnostics).

## 4. Architecture

```text
backend.rs::process_document
        │
        ▼
parsers/lockfile_resolver.rs
   ├─ trait LockfileResolver           (#[async_trait])
   ├─ select_resolver(file_type, manifest_path, manifest_content) -> Option<Box<dyn LockfileResolver>>
   └─ resolve_versions_from_lockfile(deps, resolver, manifest_path) -> Option<Arc<LockfileGraph>>
        │
        ▼
parsers/{cargo_lock,npm_lock,python_lock,go_sum,composer_lock,pubspec_lock,packages_lock_json,gemfile_lock}.rs
   └─ pub struct {Cargo,Npm,Python,Go,Php,Dart,Csharp,Ruby}Resolver + impl LockfileResolver
```

## 5. Components

### 5.1 Trait

Located at `dependi-lsp/src/parsers/lockfile_resolver.rs`.

```rust
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use crate::parsers::Dependency;
use crate::parsers::lockfile_graph::LockfileGraph;

#[async_trait]
pub trait LockfileResolver: Send + Sync {
    /// Locate lockfile relative to manifest path. None = no lockfile present.
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf>;

    /// Parse lockfile contents into graph. Empty graph on parse failure.
    fn parse_graph(&self, lock_content: &str) -> LockfileGraph;

    /// Normalize package name for resolution lookup.
    /// Default: identity. Override for PEP 503 (Python), lowercase (Ruby/NuGet/Composer).
    fn normalize_name(&self, name: &str) -> String {
        name.to_string()
    }

    /// Resolve the version for one dependency from a parsed graph.
    /// Default: first-wins lookup with `normalize_name` applied to BOTH the
    /// dependency name and each `LockfilePackage.name`. Synchronous —
    /// implementations operate on the already-loaded `LockfileGraph`.
    /// Override for ecosystems whose lockfile records multiple versions of the
    /// same package (e.g., Cargo workspaces with root-package disambiguation,
    /// Go modules with multiple recorded versions per module path).
    fn resolve_version(&self, dep: &Dependency, graph: &LockfileGraph) -> Option<String> {
        let normalized = self.normalize_name(&dep.name);
        graph
            .packages
            .iter()
            .find(|p| self.normalize_name(&p.name) == normalized)
            .map(|p| p.version.clone())
    }
}
```

**Dispatch**: `Box<dyn LockfileResolver>` via `async_trait` macro. Pattern matches existing `AdvisoryReadCache`/`AdvisoryWriteCache` (`src/cache/advisory/mod.rs`).

### 5.2 Resolvers

| Resolver | Module | Stored State | Notes |
|----------|--------|--------------|-------|
| `CargoResolver` | `cargo_lock.rs` | `root_package: Option<String>` | Computed via `cargo_root_package_name(content)` at selection |
| `NpmResolver` | `npm_lock.rs` | `lock_path: PathBuf, sub: NpmLockfileType` | Sub-format probed at selection via `find_npm_lockfile` |
| `PythonResolver` | `python_lock.rs` | `lock_path: PathBuf, sub: PythonLockfileType` | Preferred via `detect_python_tool`; resolved via `find_python_lockfile` |
| `GoResolver` | `go_sum.rs` | `()` | Adapts `HashMap<String, Vec<String>>` → first version per module |
| `PhpResolver` | `composer_lock.rs` | `()` | `normalize_composer_name` |
| `DartResolver` | `pubspec_lock.rs` | `()` | Identity normalize |
| `CsharpResolver` | `packages_lock_json.rs` | `()` | `normalize_nuget_name` |
| `RubyResolver` | `gemfile_lock.rs` | `()` | `normalize_gem_name` |

For `NpmResolver` and `PythonResolver`, the constructor performs the lockfile-on-disk probe, so `find_lockfile` simply returns the cached `lock_path`. This preserves the eager probing semantics of the current code.

### 5.3 Selector

```rust
pub async fn select_resolver(
    file_type: FileType,
    manifest_path: &Path,
    manifest_content: &str,
) -> Option<Box<dyn LockfileResolver>> {
    match file_type {
        FileType::Cargo => Some(Box::new(CargoResolver {
            root_package: cargo_root_package_name(manifest_content),
        })),
        FileType::Npm => {
            let (lock_path, sub) = find_npm_lockfile(manifest_path).await?;
            Some(Box::new(NpmResolver { lock_path, sub }))
        }
        FileType::Python => {
            let preferred = detect_python_tool(manifest_content);
            let (lock_path, sub) = find_python_lockfile(manifest_path, preferred).await?;
            Some(Box::new(PythonResolver { lock_path, sub }))
        }
        FileType::Go => Some(Box::new(GoResolver)),
        FileType::Php => Some(Box::new(PhpResolver)),
        FileType::Dart => Some(Box::new(DartResolver)),
        FileType::Csharp => Some(Box::new(CsharpResolver)),
        FileType::Ruby => Some(Box::new(RubyResolver)),
        FileType::Maven => None,
    }
}
```

### 5.4 Generic Helper

```rust
pub async fn resolve_versions_from_lockfile(
    dependencies: &mut [Dependency],
    resolver: Box<dyn LockfileResolver>,
    manifest_path: &Path,
) -> Option<Arc<LockfileGraph>> {
    let lock_path = resolver.find_lockfile(manifest_path).await?;
    let lock_content = match read_lockfile_capped(&lock_path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Could not read lockfile at {}: {}", lock_path.display(), e);
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
        dependencies.iter().filter(|d| d.resolved_version.is_some()).count(),
        lock_path.display()
    );
    Some(Arc::new(graph))
}
```

### 5.5 Call Site

`backend.rs::process_document` lines 162-605 collapse to:

```rust
let lockfile_graph = if let Ok(manifest_path) = uri.to_file_path() {
    if let Some(resolver) = select_resolver(file_type, &manifest_path, content).await {
        resolve_versions_from_lockfile(&mut dependencies, resolver, &manifest_path).await
    } else {
        None
    }
} else {
    None
};
```

## 6. Data Flow

1. `process_document` receives `(uri, content)`.
2. `file_type = detect_file_type(uri)` (existing).
3. `dependencies = parse(content)` (existing).
4. `manifest_path = uri.to_file_path()`.
5. `select_resolver(file_type, &manifest_path, content).await` returns `Option<Box<dyn LockfileResolver>>`.
6. `resolve_versions_from_lockfile(&mut dependencies, resolver, &manifest_path).await` returns `Option<Arc<LockfileGraph>>`.
7. `lockfile_graph` is consumed by downstream code (vuln attribution, diagnostics) — unchanged.

## 7. Error Handling

Refactor preserves existing silent-failure semantics:

| Surface | Behavior |
|---------|----------|
| `find_lockfile` returns `None` | Helper exits early; caller sets `lockfile_graph = None` |
| `read_lockfile_capped` fails | `tracing::debug!("Could not read lockfile at {}: {}", path, e)` — **identical to current code** |
| `parse_graph` parse failure | Returns empty `LockfileGraph` (existing parser behavior — `match toml::from_str { Ok(v) => v, Err(_) => return map }`) |
| Selector returns `None` | Caller treats as "no resolver", proceeds with `lockfile_graph = None` |

**Net behavior change**: zero. Same logs, same fallthrough.

## 8. Testing Strategy (TDD)

### 8.1 Layers

1. **Trait contract tests** (`parsers/lockfile_resolver.rs::tests`)
   - `select_resolver` returns a resolver of the expected type per `FileType`.
   - `select_resolver` returns `None` for `FileType::Maven`.
   - `resolve_versions_from_lockfile` populates `dep.resolved_version` from graph.
   - Empty `dependencies` slice → `Some(Arc::new(empty_graph))` (or `None` if find fails).
   - When `find_lockfile` returns `None`, helper returns `None`.

2. **Per-resolver impl tests** (in each parser module's `#[cfg(test)] mod tests`)
   - Cargo: `CargoResolver` honors `root_package` filter.
   - Npm: 4 sub-formats covered (PackageLock, Pnpm, Yarn, Bun).
   - Python: 4 sub-formats covered + PEP 503 normalization (`Foo_Bar` ↔ `foo-bar`).
   - Go: `Vec<String>` versions → first-version selection.
   - Ruby/PHP/C#: name normalization round-trips.

3. **Integration test** (`tests/lockfile_resolver_integration.rs` — new)
   - Tempdir-rooted synthetic projects per ecosystem.
   - Drives `select_resolver` + `resolve_versions_from_lockfile` end-to-end.
   - Asserts `Dependency.resolved_version` matches expectation.

4. **Regression belt**: existing parser unit tests untouched.

### 8.2 TDD Order

Per ecosystem, Red→Green→Refactor:
1. Failing test for `XResolver::find_lockfile` + `XResolver::parse_graph`.
2. Implement struct + `impl LockfileResolver`.
3. Wire into `select_resolver`.
4. Add integration test entry.
5. Run `cargo test --lib` and ecosystem-scoped integration.
6. Migrate the corresponding `process_document` block last (after all 8 resolvers are green).

### 8.3 Acceptance Criteria

- [ ] `cargo test --lib` passes.
- [ ] `cargo test --test integration_test` passes.
- [ ] `cargo test --test lockfile_resolver_integration` passes (new).
- [ ] No new `unwrap()` / `expect()` in main code (CLAUDE.md rule).
- [ ] No `#[allow(...)]` added (CLAUDE.md rule).
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `process_document` body reduced by ~400 lines.
- [ ] `CHANGELOG.md` `[Unreleased]` section updated under `Changed`.

## 9. Migration Plan

1. Land trait + selector + helper + 1 resolver (Cargo) — gated migration of Cargo block only.
2. Land remaining 7 resolvers + ecosystem-scoped tests, migrating each block as it lands.
3. Land integration test once all 8 resolvers exist.
4. Final pass removes legacy code from `process_document`; verify diff size matches estimate.

## 10. Risks

| Risk | Mitigation |
|------|------------|
| Behavior drift in graph construction | Per-resolver tests assert graph parity with pre-refactor parser output |
| Performance regression from `dyn` dispatch | I/O dominates; trait method count is minimal (3); pattern already used elsewhere (`AdvisoryReadCache`) |
| Sub-format detection diverges from current eager probing | Selector probes eagerly (same call sites: `find_npm_lockfile`, `find_python_lockfile`) |
| Async trait compile-time cost | Codebase already imports `async_trait` (see `Cargo.toml`); no new dependency |

## 11. Out of Scope

- Switching error handling to `Result<_, LockfileError>` (deferred to future RFC).
- Maven support (already unsupported in `process_document`).
- Unifying `LockfilePackage` schema across ecosystems beyond what already exists.
