# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.4.3] - 2026-02-24

### Fixed

- Handle pre-release versions in Python compatible release operator (`~=`) (#154)
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

[Unreleased]: https://github.com/mpiton/zed-dependi/compare/v1.4.3...HEAD
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
