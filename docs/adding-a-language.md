---
title: Adding a New Language
layout: default
nav_order: 10
description: "Step-by-step guide for adding support for a new package manager / ecosystem to Dependi"
---

# Adding a New Language
{: .no_toc }

Step-by-step guide to adding a new language/ecosystem to Dependi. Worked example: Swift Package Manager.
{: .fs-6 .fw-300 }

<!--
  Every fenced ```rust block in this file (without `ignore`) is mirrored as
  a doctest in `dependi-lsp/src/docs/swift_tutorial_fixture.rs`. Edits to
  the snippets MUST be reflected there or the doctests drift.
-->

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

## 1. Introduction

This guide walks you through adding support for a new language or package manager to Dependi. By the end, your fork will detect the manifest file, parse its dependencies, fetch versions from the upstream registry, surface vulnerabilities via OSV.dev, and offer the same inlay hints, diagnostics, and code actions every other supported ecosystem gets.

The worked example throughout is **Swift Package Manager** (`Package.swift`). At the time of writing, SwiftPM is not yet supported, which makes it a good candidate: you can follow the tutorial end-to-end and ship a real PR. If you target a different ecosystem, use the example as a template — the wire-up steps are identical.

### What you need before you start

- **Rust 1.94 or newer** (this repository is on edition 2024).
- **Git, Cargo, and the `wasm32-wasip1` target**: `rustup target add wasm32-wasip1`.
- **Familiarity with `async`/`await`**. Registry clients are async; parsers are synchronous.
- **A sample manifest from your target ecosystem** to drive your first test.
- **The OSV.dev ecosystem name**, if your registry is in OSV's coverage list. Look it up at <https://ossf.github.io/osv-schema/#defined-ecosystems> before starting Step 4. For SwiftPM the value the tutorial uses is `"SwiftURL"`; verify against the schema in case it has changed.

### What you'll touch

Five files (six if your ecosystem has lock files):

1. `dependi-lsp/src/file_types.rs` — file detection, ecosystem mapping, cache key.
2. `dependi-lsp/src/parsers/<your-lang>.rs` (new) plus `parsers/mod.rs` declaration.
3. `dependi-lsp/src/registries/<your-lang>.rs` (new) plus `registries/mod.rs` declaration.
4. `dependi-lsp/src/backend.rs` — `ProcessingContext` field, parser dispatch, registry dispatch.
5. `dependi-lsp/src/vulnerabilities/mod.rs` — `Ecosystem` variant + OSV string.
6. (Optional) `dependi-lsp/src/parsers/lockfile_resolver.rs` if your ecosystem has lock files.

The "Reference checklist" at the bottom of this page enumerates every individual edit so you can use it as a final review before opening your PR.

## 2. The big picture

When a user opens a manifest file, the LSP runs roughly this pipeline for every dependency:

```text
URI ──► file_types::FileType::detect ──► dispatch_parse ──► Vec<Dependency>
                                                              │
                                                              ▼
                                              registry.get_version_info ──► VersionInfo
                                                              │
                                                              ▼
                                                vulnerabilities::check ──► Vec<Vulnerability>
                                                              │
                                                              ▼
                                                  inlay hints / diagnostics / code actions
```

To plug a new ecosystem in, you teach each stage of that pipeline what to do with your file type. The five stages map to the five files listed in Section 1.

The two trait surfaces a contributor implements are:

```rust,ignore
// In dependi-lsp/src/parsers/mod.rs
pub trait Parser: Send + Sync {
    fn parse(&self, content: &str) -> Vec<Dependency>;
}

// In dependi-lsp/src/registries/mod.rs
#[allow(async_fn_in_trait)]
pub trait Registry: Send + Sync {
    async fn get_version_info(&self, package_name: &str)
        -> anyhow::Result<VersionInfo>;
    fn http_client(&self) -> std::sync::Arc<reqwest::Client>;
}
```

[`Parser`]: https://docs.rs/dependi-lsp/latest/dependi_lsp/parsers/trait.Parser.html
[`Registry`]: https://docs.rs/dependi-lsp/latest/dependi_lsp/registries/trait.Registry.html

`Parser` is synchronous. `Registry` is asynchronous and Send + Sync (so it can be wrapped in `Arc` and shared across the request pool). The trait uses native `async fn` rather than the `async-trait` crate; the `#[allow(async_fn_in_trait)]` attribute is needed because the trait is internal and the `Send + Sync` bound is already declared on the trait itself.

## 3. Step 1 — Define the file type

Open `dependi-lsp/src/file_types.rs`. You will make six edits.

### 3.1 Add the enum variant

Add `Swift` to the `FileType` enum (the variant order doesn't matter — alphabetical keeps diffs small):

```rust,ignore
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Cargo,
    Csharp,
    Dart,
    Go,
    Maven,
    Npm,
    Php,
    Python,
    Ruby,
    Swift,        // ← new
}
```

### 3.2 Add detection

`FileType::detect` is an `if`/`else if` chain over `path.ends_with(...)`, not a `match` on the filename. Add your branch alongside the existing ones:

```rust,ignore
impl FileType {
    pub fn detect(uri: &Url) -> Option<Self> {
        let path = uri.path();
        let filename = path.rsplit('/').next().unwrap_or(path);
        if path.ends_with("Cargo.toml") {
            Some(FileType::Cargo)
        // ... existing arms ...
        } else if path.ends_with("Package.swift") {              // ← new
            Some(FileType::Swift)
        } else {
            None
        }
    }
}
```

### 3.3 Add ecosystem mapping

Map the variant to its OSV ecosystem in `to_ecosystem`. The existing arms use the full `FileType::` / `Ecosystem::` paths (not `Self::`). Existing variant names: `CratesIo`, `Npm`, `PyPI`, `Go`, `Packagist`, `Pub`, `NuGet`, `RubyGems`, `Maven`. Add your new pair the same way:

```rust,ignore
impl FileType {
    pub fn to_ecosystem(self) -> Ecosystem {
        match self {
            FileType::Cargo => Ecosystem::CratesIo,
            // ... existing arms ...
            FileType::Swift => Ecosystem::SwiftPM,             // ← new (add to Ecosystem too)
        }
    }
}
```

You'll need to add `SwiftPM` to the `Ecosystem` enum in `dependi-lsp/src/vulnerabilities/mod.rs` — Step 4 covers that edit.

### 3.4 Add the registry URL formatter, registry name, and cache key

`fmt_registry_package_url` and `fmt_cache_key` both return `impl fmt::Display + fmt::Debug` via the `fmt::from_fn` helper, so each new arm is a `write!(f, ...)` call rather than a `format!(...)` expression. `registry_name` returns `&'static str`. Three additions:

```rust,ignore
impl FileType {
    pub fn fmt_registry_package_url(self, name: &str) -> impl fmt::Display + fmt::Debug {
        fmt::from_fn(move |f| match self {
            FileType::Cargo => write!(f, "https://crates.io/crates/{name}"),
            // ... existing arms ...
            FileType::Swift => write!(f, "https://swiftpackageindex.com/{name}"),
        })
    }

    pub fn registry_name(self) -> &'static str {
        match self {
            FileType::Cargo => "crates.io",
            // ... existing arms ...
            FileType::Swift => "Swift Package Index",
        }
    }

    pub fn fmt_cache_key(self, package_name: &str) -> impl fmt::Display + fmt::Debug {
        fmt::from_fn(move |f| match self {
            FileType::Cargo => write!(f, "crates:{package_name}"),
            // ... existing arms ...
            FileType::Swift => write!(f, "swift:{package_name}"),
        })
    }
}
```

### 3.5 Verify

Add a unit test in `file_types.rs` (under the existing `#[cfg(test)] mod tests`). Note that `fmt_cache_key` returns an `impl Display`, so call `.to_string()` on it (or use the `cache_key` convenience wrapper):

```rust,ignore
#[test]
fn detects_package_swift() {
    let uri = Url::parse("file:///proj/Package.swift").unwrap();
    assert_eq!(FileType::detect(&uri), Some(FileType::Swift));
    assert_eq!(FileType::Swift.registry_name(), "Swift Package Index");
    assert_eq!(
        FileType::Swift.cache_key("swift-argument-parser"),
        "swift:swift-argument-parser"
    );
}
```

Run it:

```bash
cd dependi-lsp
cargo test file_types::tests::detects_package_swift
```

Expected: `1 passed`. If the test does not yet pass, your variant or match arm is missing.

## 4. Step 2 — Write the parser

Create `dependi-lsp/src/parsers/swift.rs` and declare it in `parsers/mod.rs` with `pub mod swift;`.

### 4.1 Span semantics — read this first

`Span` covers the **inner bytes of a token**, measured from the start of the line, end-exclusive:

```text
    .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
                  ^                                              ^         ^     ^
                  inner start                                inner end  v.start v.end

name_span    = Span { line: 4, line_start: 18, line_end: 71 }
version_span = Span { line: 4, line_start: 80, line_end: 85 }
```

If you accidentally include the surrounding quotes, LSP quick-fix code actions will replace `"1.3.0"` with `"1.4.0""` — broken. The first thing your tests should assert is that spans don't include the quotes.

### 4.2 Test first (TDD)

Add the failing test before any implementation. In `dependi-lsp/src/parsers/swift.rs`:

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::Parser;

    const SAMPLE: &str = r#"
let package = Package(
    name: "MyApp",
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
        .package(url: "https://github.com/apple/swift-log", exact: "1.5.3"),
    ]
)
"#;

    #[test]
    fn parses_two_dependencies() {
        let parser = SwiftParser::new();
        let deps = parser.parse(SAMPLE);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "swift-argument-parser");
        assert_eq!(deps[0].version, "1.3.0");
        assert_eq!(deps[1].name, "swift-log");
        assert_eq!(deps[1].version, "1.5.3");
    }

    #[test]
    fn version_span_excludes_quotes() {
        let parser = SwiftParser::new();
        let deps = parser.parse(SAMPLE);
        let line_5 = SAMPLE.lines().nth(4).unwrap();
        let inner = &line_5[deps[0].version_span.line_start as usize
            ..deps[0].version_span.line_end as usize];
        assert_eq!(inner, "1.3.0");
        assert!(!inner.starts_with('"') && !inner.ends_with('"'));
    }
}
```

Run it — it should fail to compile (`SwiftParser` doesn't exist):

```bash
cd dependi-lsp
cargo test parsers::swift
```

Expected: compilation error mentioning `cannot find type SwiftParser`.

### 4.3 Implement

Replace the rest of `dependi-lsp/src/parsers/swift.rs` body with the implementation. The doctest [Example 3](#example-3--implementing-the-parser-trait) on this page contains a complete implementation you can copy. The full file:

```rust,ignore
//! `Package.swift` parser for Swift Package Manager.

use crate::parsers::{Dependency, Parser, Span};

#[derive(Debug, Default)]
pub struct SwiftParser;

impl SwiftParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for SwiftParser {
    fn parse(&self, content: &str) -> Vec<Dependency> {
        // Body identical to Example 3 of `swift_tutorial_fixture.rs`.
        // See the doctest for the worked-out logic; this comment exists so
        // a reader doesn't read past it expecting more code.
        unimplemented!("copy the body from Example 3");
    }
}

#[cfg(test)]
mod tests { /* defined above */ }
```

> **In a real PR**, replace the `unimplemented!()` body with the parsing logic from Example 3 verbatim. Keeping the two in sync is the contributor's responsibility — the doctest catches API drift but not logic drift.

### 4.4 Run the tests

```bash
cd dependi-lsp
cargo test parsers::swift
```

Expected: `2 passed`.

### 4.5 If your manifest format is more complex

Some ecosystems use full programming languages as manifests (Swift DSL, Gradle Kotlin DSL). Naïve substring parsing covers ~95% of real-world manifests but breaks on, for example:

- Multi-line `.package(...)` calls.
- `.package(name: "X", url: "Y", ...)` with the `name:` argument.
- Dependencies inside `#if swift(>=5.5)` conditional blocks.

For those cases, study the existing `dependi-lsp/src/parsers/maven.rs` (which uses `quick-xml`) or `dependi-lsp/src/parsers/python.rs` (which uses `taplo`) for richer parsing patterns. Adding a real Swift tokenizer is out of scope for the v1 tutorial.

## 5. Step 3 — Write the registry client

Create `dependi-lsp/src/registries/swift_package_index.rs` and declare it in `registries/mod.rs` with `pub mod swift_package_index;`.

### 5.1 Construct from the shared HTTP client

Every registry takes an `Arc<reqwest::Client>` so connection pooling is shared across the LSP. Use `registries::http_client::create_shared_client()` for the default.

```rust,ignore
use std::sync::Arc;

use reqwest::Client;

use crate::registries::{Registry, VersionInfo, http_client::create_shared_client};

pub struct SwiftPackageIndexRegistry {
    client: Arc<Client>,
    base_url: String,
}

impl SwiftPackageIndexRegistry {
    pub fn with_client(client: Arc<Client>) -> Self {
        Self {
            client,
            base_url: "https://swiftpackageindex.com/api/packages".to_string(),
        }
    }
}

impl Default for SwiftPackageIndexRegistry {
    fn default() -> Self {
        Self::with_client(
            create_shared_client().expect("failed to create HTTP client"),
        )
    }
}
```

### 5.2 Implement the trait

The `Registry` trait uses native `async fn` (`#[allow(async_fn_in_trait)]` on the trait declaration) rather than the `async-trait` crate, so the `impl` block does **not** carry an `#[async_trait]` attribute.

```rust,ignore
impl Registry for SwiftPackageIndexRegistry {
    async fn get_version_info(
        &self,
        package_name: &str,
    ) -> anyhow::Result<VersionInfo> {
        let url = format!("{}/{}", self.base_url, package_name);
        let response = self.client.get(&url).send().await?;
        anyhow::ensure!(
            response.status().is_success(),
            "Swift Package Index returned {}",
            response.status()
        );
        let payload: SpiPackage = response.json().await?;
        Ok(VersionInfo {
            latest: payload.latest_version.clone(),
            versions: payload.versions.clone(),
            description: payload.summary,
            homepage: payload.url,
            repository: payload.url_alt,
            license: payload.license,
            ..Default::default()
        })
    }

    fn http_client(&self) -> Arc<Client> {
        Arc::clone(&self.client)
    }
}

#[derive(serde::Deserialize)]
struct SpiPackage {
    latest_version: Option<String>,
    versions: Vec<String>,
    summary: Option<String>,
    url: Option<String>,
    url_alt: Option<String>,
    license: Option<String>,
}
```

The `..Default::default()` spread fills the remaining `VersionInfo` fields (`yanked`, `deprecated`, `release_dates`, `vulnerabilities`, `transitive_vulnerabilities`, `latest_prerelease`, `yanked_versions`) with their defaults.

### 5.3 Test

Use `wiremock` (already in `[dev-dependencies]`) to stub the API:

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetches_latest_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/packages/apple/swift-argument-parser"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "latest_version": "1.3.0",
                "versions": ["1.0.0", "1.3.0"],
                "summary": "Argument parser",
                "url": "https://github.com/apple/swift-argument-parser",
                "url_alt": null,
                "license": "Apache-2.0"
            })))
            .mount(&server)
            .await;

        let registry = SwiftPackageIndexRegistry {
            client: std::sync::Arc::new(reqwest::Client::new()),
            base_url: format!("{}/api/packages", server.uri()),
        };
        let info = registry
            .get_version_info("apple/swift-argument-parser")
            .await
            .unwrap();
        assert_eq!(info.latest.as_deref(), Some("1.3.0"));
        assert_eq!(info.versions.len(), 2);
    }
}
```

Run it:

```bash
cd dependi-lsp
cargo test registries::swift_package_index
```

Expected: `1 passed`.

### 5.4 Pay attention to rate limits

Most registries publish a fair-use limit. Swift Package Index's API is CDN-cached and has no documented hard limit, so no client-side throttling is needed. If your target registry is strict (crates.io, for example, enforces 1 request/second), adopt the pattern in `dependi-lsp/src/registries/crates_io.rs` (look for the `RateLimiter` struct) — never burst-fire a registry.

## 6. Step 4 — Wire into the backend

Open `dependi-lsp/src/backend.rs`. The wiring is mechanical but easy to forget partial steps. Each sub-step ends with a `cargo check` to confirm the next step is set up correctly.

### 6.1 Import

At the top of `backend.rs`, add:

```rust,ignore
use crate::parsers::swift::SwiftParser;
use crate::registries::swift_package_index::SwiftPackageIndexRegistry;
```

```bash
cd dependi-lsp
cargo check
```
Expected: a warning about unused imports (you'll fix it in 6.2). No errors.

### 6.2 Add fields to `ProcessingContext`

In the `ProcessingContext` struct, add two `Arc<...>` fields:

```rust,ignore
pub(crate) struct ProcessingContext {
    // ... existing fields ...
    pub(crate) swift_parser: Arc<SwiftParser>,
    pub(crate) swift_registry: Arc<SwiftPackageIndexRegistry>,
}
```

### 6.3 Initialize them in `DependiBackend::new` (or `with_http_client`)

Wherever `ProcessingContext` is constructed, add:

```rust,ignore
swift_parser: Arc::new(SwiftParser::new()),
swift_registry: Arc::new(SwiftPackageIndexRegistry::with_client(Arc::clone(&http_client))),
```

```bash
cd dependi-lsp
cargo check
```
Expected: zero errors.

### 6.4 Dispatch in `parse_document`

In the `match FileType::detect(uri)` arm:

```rust,ignore
Some(FileType::Swift) => self.swift_parser.parse(content),
```

### 6.5 Dispatch in the registry-fetch loop

Find the `match file_type` block that calls `get_version_info`. Add:

```rust,ignore
FileType::Swift => self.swift_registry.get_version_info(name).await,
```

### 6.6 Add the `Ecosystem` variant

In `dependi-lsp/src/vulnerabilities/mod.rs`:

```rust,ignore
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Crates,
    // ... existing variants ...
    SwiftPM,
}

impl Ecosystem {
    pub fn as_osv_str(&self) -> &'static str {
        match self {
            // ... existing arms ...
            Self::SwiftPM => "SwiftURL",  // verify against the OSV schema
        }
    }
}
```

### 6.7 Verify the whole pipeline compiles

```bash
cd dependi-lsp
cargo check
cargo test
```

Expected: zero errors. Test count increases by 3 (the two parser tests and the one registry test you wrote in Steps 2 and 3).

## 7. Step 5 — (Optional) Lockfile resolver

If your ecosystem has a lockfile (`Package.resolved` for SwiftPM, `pnpm-lock.yaml` for pnpm, etc.), Dependi can pin diagnostics to the lock-resolved version instead of the manifest range. Skip this section for the SwiftPM v1 walkthrough — it's a good follow-up issue.

### 7.1 Implement the trait

In a new `dependi-lsp/src/parsers/swift_resolved.rs`:

```rust,ignore
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::parsers::{Dependency, lockfile_graph::LockfileGraph,
                     lockfile_resolver::LockfileResolver};

pub struct SwiftLockfileResolver;

#[async_trait]
impl LockfileResolver for SwiftLockfileResolver {
    async fn find_lockfile(&self, manifest_path: &Path) -> Option<PathBuf> {
        let lockfile = manifest_path.parent()?.join("Package.resolved");
        if tokio::fs::try_exists(&lockfile).await.unwrap_or(false) {
            Some(lockfile)
        } else {
            None
        }
    }

    fn parse_graph(&self, lock_content: &str) -> LockfileGraph {
        // Parse Package.resolved (JSON v2 format) and return a graph.
        // See dependi-lsp/src/parsers/cargo_lock.rs for a complete example.
        let _ = lock_content;
        LockfileGraph::default()
    }

    fn resolve_version(
        &self,
        dep: &Dependency,
        graph: &LockfileGraph,
    ) -> Option<String> {
        graph.packages.iter()
            .find(|p| p.name == dep.name)
            .map(|p| p.version.clone())
    }
}
```

### 7.2 Register the resolver

In `dependi-lsp/src/parsers/lockfile_resolver.rs`, extend `select_resolver`:

```rust,ignore
pub fn select_resolver(file_type: FileType)
    -> Option<Box<dyn LockfileResolver>>
{
    match file_type {
        FileType::Cargo => Some(Box::new(crate::parsers::cargo_lock::CargoLockResolver)),
        // ... existing arms ...
        FileType::Swift => Some(Box::new(crate::parsers::swift_resolved::SwiftLockfileResolver)),
        _ => None,
    }
}
```

### 7.3 Verify

```bash
cd dependi-lsp
cargo test parsers::swift_resolved
```

Expected: `1 passed` (assuming you wrote a test). Without this resolver, vulnerability scanning still works on the declared range, but transitive vulnerabilities won't be detected.

## 8. Step 6 — Update docs and CHANGELOG

_TBD — Task 17._

## 9. Verifying your work

_TBD — Task 18._

## 10. Reference checklist

_TBD — Task 19._

## 11. Common pitfalls

_TBD — Task 20._
