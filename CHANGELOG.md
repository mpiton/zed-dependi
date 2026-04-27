# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cache RustSec advisory details fetched from OSV.dev with a hybrid
  memory + SQLite layer. `check_rustsec_unmaintained` now consults the
  cache before issuing per-advisory `GET /vulns/{id}` requests, with a
  24-hour TTL for found advisories and a 1-hour TTL for negative entries
  (404 from OSV). The cache lives in a separate SQLite database
  (`~/.cache/dependi/advisory_cache.db`) and is configurable via the new
  `AdvisoryCacheConfig`. Drastically reduces redundant network requests
  during repeated Rust dependency scans
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- `Ignore package "<name>"` quick-fix code action on every dependency. Adds
  the package to `lsp.dependi.initialization_options.ignore` in the workspace
  `.zed/settings.json`, creating the file if it does not exist, deduplicating
  if the package is already listed, and preserving any other settings
  ([#226](https://github.com/mpiton/zed-dependi/issues/226))
- Diagnostics and "Update" code actions now respect the `ignore` list
  (previously only inlay hints did), giving consistent silencing behavior for
  packages users have chosen to ignore
- `--output html` format for `dependi-lsp scan` — self-contained HTML report
  with summary table and separate Direct/Transitive sections, HTML-escaped
  against hostile advisory content, suitable for CI/CD artifacts
  ([#225](https://github.com/mpiton/zed-dependi/issues/225))
- Lockfile-based vulnerability scanning in the `dependi-lsp scan` CLI and in
  the LSP editor: when a manifest has an adjacent lockfile (`Cargo.lock`,
  `package-lock.json` (v2/v3 flat format; v1 legacy format not supported), `pnpm-lock.yaml` v6/v9, `yarn.lock` v1, `poetry.lock`,
  `uv.lock`, `Pipfile.lock`, `composer.lock`, `Gemfile.lock`), the scanner now
  uses the exact resolved versions **and** walks the full dependency graph to
  check transitive dependencies via OSV.dev.
- Transitive vulnerabilities are attributed to the direct dependency that
  introduces them (`npm audit` style) and surfaced in:
  - CLI reports (`summary`, `markdown`, `json`) under a distinct
    `Transitive dependencies` section, each entry tagged with the direct
    parent via `via_direct`.
  - LSP diagnostics on the direct dependency line, with the message
    summarizing up to 3 transitive CVEs (`N transitive CVE(s): pkg@ver (CVE-ID), ...`)
    and escalating severity when a transitive CVE is more critical than the
    direct ones.
  - LSP hover content, with a dedicated `Transitive vulnerabilities` section
    listing each CVE with severity, package@version, description, and link.
- `--no-use-lockfile` flag on `dependi-lsp scan` to disable lockfile detection
  for debugging or when only the manifest should be inspected.
- `VersionInfo.transitive_vulnerabilities: Vec<TransitiveVuln>` and new public
  `TransitiveVuln` type in `registries` for consumers of the public API.
- Support for Java/Maven projects (pom.xml):
  - Parse direct dependencies with `${properties}` substitution
  - Scope awareness (`test`/`provided` marked as dev dependencies)
  - Fetch versions and metadata from Maven Central (`maven-metadata.xml` + best-effort POM)
  - Vulnerability scanning via OSV.dev (Maven ecosystem)

### Changed

- Refactored `process_document` lockfile resolution into a new
  `LockfileResolver` trait + 8 ecosystem implementations (Cargo, npm,
  Python, Go, PHP, Dart, C#, Ruby). Removes ~440 lines of duplicated
  code from `backend.rs` and routes all ecosystems through a single
  `select_resolver` + `resolve_versions_from_lockfile` code path. Behavior is preserved (silent failure on parse error, identical
  `tracing::debug!` logs, `lockfile_graph` still populated for downstream
  vulnerability attribution) and Cargo multi-version disambiguation via
  `[package].name` is upheld through a dedicated `CargoResolver::resolve_version`
  override. Adds 9 end-to-end integration tests covering all ecosystems plus
  Maven (returns `None` as expected)
  ([#239](https://github.com/mpiton/zed-dependi/issues/239))
- Cache traits (`ReadCache`, `WriteCache`) are now async (AFIT). The SQLite cache offloads blocking `rusqlite` work to `tokio::task::spawn_blocking`, keeping the LSP event loop responsive under load. Reduces tail latency on operations that hit the persistent cache. (#235)
- Bump `hashbrown` from 0.16.1 to 0.17.0 (purely additive release, hashbrown MSRV 1.85 ≤ project MSRV 1.94)
- Bump `tokio` constraint from 1.50 to 1.52 in `dependi-lsp/Cargo.toml` (lockfile resolves 1.52.1; patch + minor, backwards compatible)
- Bump `actions/github-script` from v8 to v9 in `contributor-experience.yml` workflow (Octokit v7; inline scripts unaffected — no `require()` or `getOctokit` shadowing)
- Refresh all Cargo lockfiles (`dependi-lsp`, `dependi-zed`, `dependi-lsp/fuzz`) with latest semver-compatible transitive dependencies
- Track dependency name and version lines separately
- Refactor `parse_pyproject_toml` into focused per-section helpers (`parse_pep621_deps`, `parse_pep621_optional`, `parse_poetry_main`, `parse_poetry_dev_legacy`, `parse_poetry_groups`, `parse_pep735_groups`, `parse_hatch_envs`) and resolve dependency positions via taplo `text_range()` instead of repeated full-file line scans. Improves readability and eliminates O(deps × lines) scanning. ([#240](https://github.com/mpiton/zed-dependi/issues/240))

### Fixed

- Poetry inline-table dependencies (`name = { version = "...", optional = true }`) now correctly propagate the `optional = true` flag to the parsed `Dependency`. Previously the flag was silently dropped. ([#240](https://github.com/mpiton/zed-dependi/issues/240))
- `parse_package_lock_graph` no longer surfaces nested `node_modules/<a>/node_modules/<b>`
  copies. With the new graph-based resolver path, transitive nested entries
  could shadow the top-level direct dependency on a first-wins lookup and
  return the wrong version. The graph parser now reuses the same
  `extract_name_from_node_modules_path` helper as the flat parser, which
  skips nested entries
  ([#239](https://github.com/mpiton/zed-dependi/issues/239))
- `LockfileResolver::resolve_version` (default impl) now applies
  `normalize_name` to BOTH the dependency name and each `LockfilePackage.name`
  so resolvers whose `parse_graph` does not pre-normalize entries (e.g.,
  Composer, NuGet, Ruby) still match correctly
- `GoResolver::resolve_version` deduplicates identical version strings before
  the ambiguity check; repeated entries no longer force the result to `None`
  when only a single unique version is present
- Abort the previous advisory cache cleanup tasks when `initialize` rebuilds
  the runtime, instead of leaking them. `spawn_default_cleanup_task` now
  returns the `JoinHandle`, the backend tracks the handles in a `Mutex`, and
  reconfiguration drains-and-aborts before installing the new caches; without
  this the old tasks kept ticking forever holding `Arc` clones of the
  replaced memory/SQLite layers
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Fall through to the negative cache and network when the positive
  advisory cache returns `NotFound`, instead of treating it as
  authoritative. Pre-split builds wrote 404s to the same SQLite layer with
  the long positive TTL; without this fall-through, an upgrading user
  would otherwise be stuck with 24 h-stale `NotFound` rows that never
  retry. Refreshing 200 responses overwrite the stale row via the
  existing UPSERT
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Make `OsvClient::with_endpoint` fallible (`anyhow::Result<Self>`) and
  drop its `reqwest::Client::new()` fallback. `Client::new()` is itself
  `Client::builder().build().expect(...)` and panics under the same
  conditions as the failing `build()`, so the fallback was an illusion.
  The scan CLI handler logs and returns a non-zero exit code instead of
  panicking ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Reuse the LSP's shared `reqwest::Client` for the `OsvClient` instead of
  building a second one. The new `OsvClient::with_shared_client_and_caches`
  takes the already-verified `Arc<Client>` so there is no second TLS
  builder failure point at startup, and `build_advisory_runtime` is
  genuinely infallible
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Reconfigure the advisory cache trio (`advisory_cache`,
  `negative_advisory_cache`, `osv_client`) inside `LanguageServer::initialize`
  once the LSP receives client settings. Previously the caches were built
  from `Config::default()` at struct-construction time and never replaced,
  so user overrides for `db_path`, `ttl_secs`, `negative_ttl_secs`, or
  `enabled` had no runtime effect even though the wiring through
  `HybridAdvisoryCache::from_config` itself was correct
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Create parent directories for a user-supplied `AdvisoryCacheConfig.db_path`
  before opening SQLite. Without this, a nested override silently degraded
  to the in-memory layer because `SqliteConnectionManager` could not find
  the directory ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Drop writes when the advisory memory cache is configured with a zero TTL
  (i.e. `enabled = false`). Previously disabled-cache mode still inserted
  entries that were immediately expired but kept occupying memory until
  the cleanup task ran ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Offload `MemoryAdvisoryCache::cleanup_expired` from the cleanup loop
  through `tokio::task::spawn_blocking` so a `DashMap::retain` over a very
  large cache cannot stall the runtime
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Replace the flaky unwritable-path advisory cache test with a deterministic
  blocker file inside a `tempdir`, removing the dependency on
  `/this/path/should/not/exist/...` succeeding to fail
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Isolate the advisory cache wiring smoke test behind a `tempdir`-scoped
  `db_path` so a previously persisted `RUSTSEC-2020-0036` row in the user's
  default cache cannot turn the miss assertion into a flake
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Wire `AdvisoryCacheConfig` through `HybridAdvisoryCache::from_config` so
  user settings (`enabled`, `ttl_secs`, `negative_ttl_secs`, `db_path`)
  actually take effect; previously the backend instantiated the cache via
  `HybridAdvisoryCache::new()` and ignored every config field
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Apply `negative_ttl_secs` (default 1 h) to OSV 404 responses; they were
  previously cached on the same 24 h schedule as positive entries, so a
  brand-new RUSTSEC ID that OSV had not yet ingested stayed hidden for a
  full day. 404s now live in a separate memory-only negative cache
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Preserve the remaining TTL when backfilling the L1 advisory cache from
  L2: previously every L2 hit re-stamped the memory entry with
  `Instant::now()` and the full TTL, doubling the effective lifetime for
  entries already near SQLite expiry
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Validate advisory IDs with an ASCII-alphanumeric/`-` whitelist (≤64 chars)
  before interpolation into URLs and use as cache keys, blocking malformed
  values returned in OSV responses
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- Drop the unusable `idx_advisory_expiry` index from the advisory schema:
  the cleanup query (`WHERE inserted_at + ttl_secs * ? < ?`) cannot use a
  composite index because of the arithmetic expression
  ([#237](https://github.com/mpiton/zed-dependi/issues/237))
- `dependi-lsp scan` now uses `effective_version()` (the resolved lockfile
  version when available) instead of the declared version specifier when
  querying OSV — previously the CLI queried using the specifier string (e.g.
  `^1.0`), which caused false negatives.
- Respect the `package` field of `Cargo.toml` dependencies
- `settings_edit::build_create_edit` now uses a full-document replace range
  instead of a zero-width insert at `(0,0)`. With `ignore_if_exists: true`,
  if `.zed/settings.json` appeared between read and apply the `CreateFile`
  would be skipped and the text edit would prepend new JSON to the existing
  file, corrupting it ([#226](https://github.com/mpiton/zed-dependi/issues/226))
- `find_workspace_root` now returns `None` when no ancestor contains `.zed/`
  instead of falling back to the manifest's parent directory. Prevents a
  stray `.zed/settings.json` being written to a nested directory when the
  real Zed workspace lives higher up; the Ignore code action degrades
  gracefully when no workspace root can be resolved
- `Backend::code_action` no longer holds a `DashMap` guard across `.await`
  boundaries; it now snapshots document state into owned values and drops
  the guard before any async I/O to avoid potential deadlocks
- `Ignore package` quick-fix now emitted for Maven property-reference
  dependencies (e.g., `${spring.version}`). These deps remain ineligible
  for the `Update` action (which would break property-driven version
  management) but can still be silenced. Also restored the pre-refactor
  behavior that `Update all N dependencies` considers every non-ignored
  dep in the file rather than only those inside the requested range
- Bare PEP 440 pre-release versions (e.g., `4.0.0a6`, `1.0b2`, `2.0rc1`,
  `1.0.0.dev1`) no longer trigger spurious downgrade suggestions in
  `compare_versions`. This scenario occurred with Python lockfile resolution
  (poetry/uv/pdm/pipenv) pinning a direct requirement to a 4.x pre-release
  while PyPI's latest stable was a lower major (e.g., apscheduler 3.11.x).
  `compare_versions` now retries semver parsing after stripping PEP 440
  pre-release markers when the initial parse fails
  ([#154](https://github.com/mpiton/zed-dependi/issues/154)).

### Security

- Limit concurrent OSV RustSec advisory detail requests to 5 per batch to avoid
  unbounded request bursts and reduce OSV API rate-limit risk
  ([#229](https://github.com/mpiton/zed-dependi/issues/229)).
- Lockfile reads are now capped at 50 MiB to prevent out-of-memory on hostile
  or corrupted inputs (both in CLI and LSP backend).
- Bump transitive `rand` from 0.9.2 to 0.9.4 via `cargo update` to address [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html) / [GHSA-cq8v-f236-94qc](https://github.com/rust-random/rand/security/advisories/GHSA-cq8v-f236-94qc) — unsoundness when the `log` and `thread_rng` features are combined with a custom logger that calls `rand::rng()` during a reseed cycle
- Bump transitive `rustls-webpki` from 0.103.10 to 0.103.12 via `cargo update` to address [RUSTSEC-2026-0098](https://rustsec.org/advisories/RUSTSEC-2026-0098.html) / [GHSA-965h-392x-2mh5](https://github.com/rustls/webpki/security/advisories/GHSA-965h-392x-2mh5) (URI name constraints ignored) and [RUSTSEC-2026-0099](https://rustsec.org/advisories/RUSTSEC-2026-0099.html) / [GHSA-xgp8-3hg3-c2mh](https://github.com/rustls/webpki/security/advisories/GHSA-xgp8-3hg3-c2mh) (wildcard DNS name constraints bypass)
- Reject non-`http(s)` repository and homepage URLs from npm and
  Packagist package metadata. The previous substring-based
  `normalize_repo_url` allowed `ssh`, `git+ssh`, `ftp`, `file`,
  `javascript`, `data`, `mailto`, and other schemes to pass through to
  the IDE; the `homepage` field was unsanitized entirely. The new
  shared `sanitize_repo_url` and `sanitize_external_url` helpers
  (backed by the `url` crate) drop any URL whose scheme falls outside
  the `{http, https}` allowlist, contain embedded credentials, or
  collapse to a bare host after stripping a trailing `.git`
  ([#230](https://github.com/mpiton/zed-dependi/issues/230), CWE-20).

## [1.7.0] - 2026-04-07

### Added

- Add support for PEP 735 `[dependency-groups]` in `pyproject.toml` — versioned dependencies are parsed, `include-group` references and unversioned items are skipped ([#219](https://github.com/mpiton/zed-dependi/pull/219))
- Add support for Hatch environment dependencies in `pyproject.toml` (`[tool.hatch.envs.*]`) and `hatch.toml` (`[envs.*]`), parsing both `dependencies` and `extra-dependencies` ([#220](https://github.com/mpiton/zed-dependi/pull/220))

### Changed

- Bump `sha2` from 0.10 to 0.11 in dependi-zed (digest 0.11 migration)
- Bump `actions/configure-pages` from v5 to v6 and `actions/deploy-pages` from v4 to v5 in CI
- Update all Cargo lockfiles with latest compatible dependency versions

### Security

- Bump `requests` from 2.32.4 to 2.33.0 in Python fuzz corpus (`dependi-lsp/fuzz/corpus/fuzz_python/requirements.txt`) — insecure temp file reuse in `extract_zipped_paths()` ([#213](https://github.com/mpiton/zed-dependi/pull/213))

## [1.6.1] - 2026-03-25

### Fixed

- Fix false-positive "update available" diagnostic when `Cargo.lock` contains multiple versions of the same crate (e.g., `hashbrown 0.15.5` pulled by a transitive dep and `hashbrown 0.16.1` used directly). The root package's `dependencies` list is now used to select the correct locked version ([#210](https://github.com/mpiton/zed-dependi/issues/210))

## [1.6.0] - 2026-03-24

### Added

- Add Node.js lockfile version resolution to eliminate false-positive "update available" warnings for `package.json` dependencies ([#186](https://github.com/mpiton/zed-dependi/issues/186))
  - `package-lock.json` (npm lockfileVersion 1, 2, and 3)
  - `yarn.lock` (Yarn Classic v1 and Yarn Berry v2+)
  - `pnpm-lock.yaml` (pnpm v6 and v9)
  - `bun.lock` (Bun text JSONC format)
- Add Python lockfile version resolution to eliminate false-positive "update available" warnings for `pyproject.toml` and `requirements.txt` dependencies ([#186](https://github.com/mpiton/zed-dependi/issues/186))
  - `poetry.lock` (Poetry)
  - `uv.lock` (uv)
  - `pdm.lock` (PDM)
  - `Pipfile.lock` (Pipenv)
- Add Go lockfile version resolution to eliminate false-positive "update available" warnings for `go.mod` dependencies ([#186](https://github.com/mpiton/zed-dependi/issues/186))
  - `go.sum` (Go modules checksum database)
- Add PHP lockfile version resolution to eliminate false-positive "update available" warnings for `composer.json` dependencies ([#186](https://github.com/mpiton/zed-dependi/issues/186))
  - `composer.lock` (Composer)
- Add Dart lockfile version resolution (`pubspec.lock`) to eliminate false-positive update warnings
- Add C# lockfile version resolution (`packages.lock.json`) to eliminate false-positive update warnings
- Add Ruby lockfile version resolution (`Gemfile.lock`) to eliminate false-positive update warnings

### Changed

- Bump MSRV from 1.85 to 1.94; adopt stable let-chains, `fmt::from_fn` for
  zero-allocation display formatting, and inlined format args across the codebase
- Removed `String`-returning formatting functions as deprecated
- Use `hashbrown::Hash{Map, Set}` instead of the `std::collections::Hash{Map, Set}`,
  to enable more flexible usage and reduce allocations. Note: `hashbrown` uses `foldhash` by
  default, instead of the `std`'s default --- SipHash.
- Update `toml` 1.0.6 → 1.0.7
- Update transitive dependencies via `cargo update`

### Fixed

- Fix false-positive "update available" reports when using minimal version syntax (e.g., `bon = "3.9"`) by reading resolved versions from `Cargo.lock` ([#184](https://github.com/mpiton/zed-dependi/issues/184))
- Fix false-positive vulnerability reports by normalizing version operators before OSV.dev queries ([#181](https://github.com/mpiton/zed-dependi/issues/181))
- Use async I/O for lockfile discovery to avoid blocking the Tokio executor
- Fix GLIBC compatibility on older Linux systems (Ubuntu 22.04, WSL) by targeting GLIBC 2.17 with cargo-zigbuild ([#198](https://github.com/mpiton/zed-dependi/issues/198))
- Use `env::var_os` instead of `env::var` for `CARGO_HOME` to avoid failures on non-UTF-8 paths
- Fix hardcoded "crates.io" in yanked version diagnostics — now uses the correct registry name/URL for all ecosystems ([#201](https://github.com/mpiton/zed-dependi/issues/201))
- Fix future timestamps rendering as negative age (e.g., "-5 hours ago") due to clock skew — now shows "just now" ([#201](https://github.com/mpiton/zed-dependi/issues/201))
- Fix hover panel showing manifest version specifier instead of resolved lockfile version ([#201](https://github.com/mpiton/zed-dependi/issues/201))
- Fix `fmt_truncate_string` emitting "..." (3 chars) even when `max_chars` < 3 ([#201](https://github.com/mpiton/zed-dependi/issues/201))

### Security

- Update `rustls-webpki` 0.103.9 → 0.103.10 in fuzz lockfile (fixes certificate revocation enforcement bug, [GHSA-pwjx-qhcg-rvj4](https://github.com/rustls/webpki/security/advisories/GHSA-pwjx-qhcg-rvj4))
- Update `aws-lc-sys` 0.38.0 → 0.39.0 in fuzz lockfile (fixes CRL Distribution Point scope check and X.509 Name Constraints bypass)

## [1.5.0] - 2026-03-16

### Added

- Add clickable links on dependency names to open package registry pages (pub.dev, crates.io, npm, PyPI, etc.) (#171)
- Add Linux ARM64 (`aarch64-unknown-linux-gnu`) release binary for devcontainers on Apple Silicon (#169)

### Changed

- Replace `r2d2_sqlite` with custom `SqliteConnectionManager` to unblock `rusqlite` upgrades (#178)
- Bump `rusqlite` from 0.38 to 0.39 (bundled SQLite 3.51.3)
- Bump `reqwest` from 0.12 to 0.13 (rustls now default TLS backend, `rustls-tls` feature renamed to `rustls`)
- Bump `chrono` from 0.4.43 to 0.4.44
- Bump `toml` from 1.0.4 to 1.0.6
- Bump `tracing-subscriber` from 0.3.22 to 0.3.23

### Removed

- Remove `r2d2_sqlite` dependency (replaced by ~50-line custom implementation)

### Fixed

- Fix pubspec.yaml dependencies with inline comments showing as outdated (false positive) (#170)

### Security

- Bump `quinn-proto` from 0.11.13 to 0.11.14 to resolve RUSTSEC-2026-0037 (DoS via QUIC transport parameters)
- Bump `time` from 0.3.45 to 0.3.47 (fix DoS via stack exhaustion, RUSTSEC-2026-0009)

## [1.4.4] - 2026-03-05

### Fixed

- Disable ANSI escape sequences in LSP log output (#162)

### Changed

- Bump `actions/upload-artifact` from v6 to v7
- Bump `actions/download-artifact` from v7 to v8
- Bump `tokio` from 1.49 to 1.50
- Bump `toml` from 1.0.3 to 1.0.4

## [1.4.3] - 2026-02-24

### Fixed

- Handle pre-release versions in Python-compatible release operator (`~=`) (#154)
- Improve detection of Python requirements and constraints files

### Changed

- CI: grant `checks:write` permission to security-audit job

## [1.4.2] - 2026-02-23

### Fixed

- Handle Python-compatible release operator (`~=`) correctly in requirements.txt and pyproject.toml (#151)

### Security

- Bump `time` from 0.3.45 to 0.3.47 (fix DoS via stack exhaustion, RUSTSEC-2026-0009)

## [1.4.1] - 2026-02-22

### Added

- Support for `zed-python-requirements` extension for Python constraints and requirements files (#148)

### Changed

- Bump `anyhow` from 1.0.101 to 1.0.102
- Bump `clap` from 4.5.57 to 4.5.60
- Bump `futures` from 0.3.31 to 0.3.32
- Bump `toml` from 0.9.11 to 1.0.3
- Bump `serial_test` from 3.2.0 to 3.2.1

## [1.4.0] - 2026-02-09

### Added

- Cargo alternative registry support for private registries (Kellnr, Cloudsmith, Artifactory, etc.) (#133)
  - Sparse index protocol implementation for querying alternative Cargo registries
  - Per-dependency registry routing via the `registry` field in `Cargo.toml`
  - Authentication via LSP configuration (environment variables) or `~/.cargo/credentials.toml` fallback
  - Registry-scoped cache keys to prevent cross-registry collisions
  - Cross-platform `CARGO_HOME` resolution (Linux, macOS, Windows)

### Changed

- Bump `anyhow` from 1.0.100 to 1.0.101
- Bump `criterion` from 0.8.1 to 0.8.2
- Update 52 transitive dependencies via cargo update

### Removed

- Remove unused `serde_yaml` dependency (deprecated since March 2024, never imported)

## [1.3.3] - 2026-02-04

### Security

- Bump `bytes` from 1.11.0 to 1.11.1 (fix integer overflow in `BytesMut::reserve`)

### Changed

- Bump `clap` from 4.5.54 to 4.5.57

## [1.3.2] - 2026-01-31

### Fixed

- pub.dev registry now returns newest version as latest instead of oldest

## [1.3.1] - 2026-01-25

### Fixed

- Update Cargo.toml versions missed in v1.3.0 release

## [1.3.0] - 2026-01-25

### Added

- Workspace dependencies parsing for Cargo.toml (#114)

### Fixed

- Python pyproject.toml parser panics by switching to taplo for TOML parsing
- Python pyproject.toml detection for `[project.*]` subsections and inline comments
- Fuzz testing crashes in parsers (#115)

### Changed

- Updated chrono from 0.4.42 to 0.4.43
- Updated thiserror from 2.0.17 to 2.0.18

## [1.2.0] - 2026-01-10

### Added

- Comprehensive private registry configuration documentation
- Authentication token support for private npm registries
- Custom registry configuration with scope-based routing
- Test coverage infrastructure with cargo-tarpaulin
- SHA256 checksum verification for LSP binary downloads
- Cargo audit in CI for dependency vulnerability scanning
- CONTRIBUTING.md with contribution guidelines
- Troubleshooting guide and FAQ section
- Comprehensive registry API documentation
- CI/CD integration documentation for CLI scan command
- Benchmark suite with criterion for performance testing
- Fuzz testing for parsers with cargo-fuzz
- GitHub Pages documentation site

### Changed

- Lazy load vulnerability checks in background for better performance
- Debounce didChange notifications to reduce processing
- Split backend.rs into domain-specific modules
- Extract is_prerelease logic into shared version_utils module
- Extract truncate_string to shared utils module
- Split Cache trait into ReadCache and WriteCache
- Enable SQLite WAL mode and r2d2 connection pooling
- Share single HTTP client across all registry clients
- Rewrite CargoParser with taplo for structured TOML parsing
- Remove #[allow(dead_code)] directives and fix warnings
- Optimize parsing performance for Go, npm, PHP, and Ruby dependency files

### Fixed

- Remove broken discussions links and Zed Discord reference
- Fix YAML syntax error in contributor-experience workflow
- Fix CI paths for dependi-lsp build and binary
- Prevent overflow in profiling and cap network iterations
- Improve profiling command safety and accuracy
- Improve test accuracy and prevent over-stripping in parsers

### Security

- Add background cache cleanup to prevent unbounded memory growth
- Add descriptive suffix to vulnerability count in inlay hints

## [1.1.0] - 2025-01-07

### Added

- Semantic version update type in code action titles (major/minor/patch indicators)
- Actionable error states for unknown version status
- Release date display in version completions
- Yanked version warnings for Cargo dependencies
- Ruby/Bundler ecosystem support (Gemfile parsing, RubyGems registry)
- "Update all" code action to update all outdated dependencies at once
- Package deprecation warnings in inlay hints and diagnostics

### Changed

- Use ASCII arrow (->) for cross-platform compatibility

## [1.0.0] - 2025-01-05

### Added

- Marketplace installation instructions in README

### Changed

- Update tokio, clap, and toml dependencies
- Update Rust dependencies and add registry compliance
- Update dependencies and fix breaking changes

## [0.3.1] - 2025-12-13

### Added

- Windows build support in CI
- Demo GIF in README
- CLI scan command
- Enable Dependabot for dependency updates

### Changed

- Bump CI actions to latest versions (checkout v6, gh-release v2, artifacts v6/v7)
- Upgrade toml 0.8 to 0.9 and rusqlite 0.32 to 0.37

### Fixed

- Multi-platform release workflow
- Dead code clippy warnings
- Clippy warnings (collapsible_if, bool_comparison)

## [0.3.0] - 2025-12-13

### Added

- Security scanning via OSV.dev API
- Vulnerability diagnostics with configurable severity levels
- Support for new ecosystems:
  - PHP (composer.json) via Packagist
  - Dart (pubspec.yaml) via pub.dev
  - .NET (*.csproj) via NuGet
- YAML and XML support for extension

## [0.2.0] - 2025-12-13

### Added

- SQLite persistent cache for version data
- User configuration via LSP initialization options
- Configurable cache TTL
- Package ignore patterns
- Inlay hints enable/disable option
- Diagnostics enable/disable option

## [0.1.1] - 2025-12-13

### Fixed

- Use 'Go Mod' language type for go.mod files in Zed

## [0.1.0] - 2025-12-13

### Added

- Initial release of dependi-lsp
- Support for 5 ecosystems:
  - Rust (Cargo.toml) via crates.io
  - Node.js (package.json) via npm
  - Python (requirements.txt, pyproject.toml) via PyPI
  - Go (go.mod) via Go Proxy
- Inlay hints showing latest version information
- Diagnostics for outdated dependencies
- Code actions for updating dependencies
- In-memory caching for version data
- Parallel registry requests (5 concurrent)

[Unreleased]: https://github.com/mpiton/zed-dependi/compare/v1.7.0...HEAD
[1.7.0]: https://github.com/mpiton/zed-dependi/compare/v1.6.1...v1.7.0
[1.6.1]: https://github.com/mpiton/zed-dependi/compare/v1.6.0...v1.6.1
[1.6.0]: https://github.com/mpiton/zed-dependi/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/mpiton/zed-dependi/compare/v1.4.4...v1.5.0
[1.4.4]: https://github.com/mpiton/zed-dependi/compare/v1.4.3...v1.4.4
[1.4.3]: https://github.com/mpiton/zed-dependi/compare/v1.4.2...v1.4.3
[1.4.2]: https://github.com/mpiton/zed-dependi/compare/v1.4.1...v1.4.2
[1.4.1]: https://github.com/mpiton/zed-dependi/compare/v1.4.0...v1.4.1
[1.4.0]: https://github.com/mpiton/zed-dependi/compare/v1.3.3...v1.4.0
[1.3.3]: https://github.com/mpiton/zed-dependi/compare/v1.3.2...v1.3.3
[1.3.2]: https://github.com/mpiton/zed-dependi/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/mpiton/zed-dependi/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/mpiton/zed-dependi/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/mpiton/zed-dependi/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/mpiton/zed-dependi/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/mpiton/zed-dependi/compare/v0.3.1...v1.0.0
[0.3.1]: https://github.com/mpiton/zed-dependi/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/mpiton/zed-dependi/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/mpiton/zed-dependi/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/mpiton/zed-dependi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/mpiton/zed-dependi/releases/tag/v0.1.0
