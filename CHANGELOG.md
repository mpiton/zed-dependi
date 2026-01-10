# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/mpiton/zed-dependi/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/mpiton/zed-dependi/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/mpiton/zed-dependi/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/mpiton/zed-dependi/compare/v0.3.1...v1.0.0
[0.3.1]: https://github.com/mpiton/zed-dependi/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/mpiton/zed-dependi/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/mpiton/zed-dependi/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/mpiton/zed-dependi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/mpiton/zed-dependi/releases/tag/v0.1.0
