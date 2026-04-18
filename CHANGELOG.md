# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Support for Java/Maven projects (pom.xml):
  - Parse direct dependencies with `${properties}` substitution
  - Scope awareness (`test`/`provided` marked as dev dependencies)
  - Fetch versions and metadata from Maven Central (`maven-metadata.xml` + best-effort POM)
  - Vulnerability scanning via OSV.dev (Maven ecosystem)

### Security

- Bump transitive `rand` from 0.9.2 to 0.9.4 via `cargo update` to address [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html) / [GHSA-cq8v-f236-94qc](https://github.com/rust-random/rand/security/advisories/GHSA-cq8v-f236-94qc) — unsoundness when the `log` and `thread_rng` features are combined with a custom logger that calls `rand::rng()` during a reseed cycle
- Bump transitive `rustls-webpki` from 0.103.10 to 0.103.12 via `cargo update` to address [RUSTSEC-2026-0098](https://rustsec.org/advisories/RUSTSEC-2026-0098.html) / [GHSA-965h-392x-2mh5](https://github.com/rustls/webpki/security/advisories/GHSA-965h-392x-2mh5) (URI name constraints ignored) and [RUSTSEC-2026-0099](https://rustsec.org/advisories/RUSTSEC-2026-0099.html) / [GHSA-xgp8-3hg3-c2mh](https://github.com/rustls/webpki/security/advisories/GHSA-xgp8-3hg3-c2mh) (wildcard DNS name constraints bypass)

### Changed

- Bump `hashbrown` from 0.16.1 to 0.17.0 (purely additive release, hashbrown MSRV 1.85 ≤ project MSRV 1.94)
- Bump `tokio` constraint from 1.50 to 1.52 in `dependi-lsp/Cargo.toml` (lockfile resolves 1.52.1; patch + minor, backwards compatible)
- Bump `actions/github-script` from v8 to v9 in `contributor-experience.yml` workflow (Octokit v7; inline scripts unaffected — no `require()` or `getOctokit` shadowing)
- Refresh all Cargo lockfiles (`dependi-lsp`, `dependi-zed`, `dependi-lsp/fuzz`) with latest semver-compatible transitive dependencies
- Track dependency name and version lines separately
- Reduce reference indirection and struct sizes
  - `Arc`-unwrap `Parser`s, as those are ZSTs
  - `Arc`-unwrap `reqwest::Client`, as it already contains `Arc`
- Make relevant doc-tests `no_run` instead of `ignore`

### Fixed

- Respect the `package` field of `Cargo.toml` dependencies

### Removed

- Redundant examples from `impl Default`s

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
