//! Dependi LSP — Language Server for dependency management.
//!
//! Provides Language Server Protocol (LSP) features for dependency manifests
//! across multiple ecosystems: Cargo, npm, PyPI, Go modules, Composer,
//! pub.dev, NuGet, Maven Central, and RubyGems. Features include version
//! inlay hints, vulnerability diagnostics, code actions for upgrades, and
//! registry-aware completion.
//!
//! # Module map
//!
//! - [`backend`]: tower-lsp [`LanguageServer`](tower_lsp::LanguageServer)
//!   implementation wiring document state to providers.
//! - [`parsers`]: Manifest and lockfile parsers per ecosystem.
//! - [`providers`]: LSP feature providers (diagnostics, inlay hints,
//!   code actions, completion, document links).
//! - [`registries`]: HTTP clients for package registries.
//! - [`vulnerabilities`]: OSV vulnerability checks plus caching.
//! - [`cache`]: Hybrid memory+SQLite version cache and advisory cache.
//! - [`auth`]: Registry credential resolution (cargo credentials, .npmrc tokens).
//! - [`config`]: User-facing settings deserialized from LSP `initialize`.
//! - [`reports`]: JSON and Markdown vulnerability report generation.
//! - [`document`]: Per-document parsed state shared across providers.
//! - [`file_types`]: File-type detection from URI and ecosystem mapping.
//! - [`settings_edit`]: `WorkspaceEdit` helpers for `.zed/settings.json` updates.
//! - [`utils`]: Shared string utilities (truncation, HTML escaping).
//!
//! # Entry point
//!
//! See [`backend::DependiBackend`] for constructing and running the server.

/// tower-lsp [`LanguageServer`](tower_lsp::LanguageServer) implementation
/// wiring document lifecycle events to parsers, registries, and providers.
pub mod backend;

/// Registry credential resolution (cargo credentials, .npmrc tokens).
pub mod auth;

/// Hybrid memory+SQLite version cache and RustSec advisory cache.
pub mod cache;

/// User-facing settings deserialized from the LSP `initialize` request.
pub mod config;

/// Per-document parsed state (dependencies, file type, lockfile graph).
pub mod document;

/// File-type detection from URI path and ecosystem-to-cache-key mapping.
pub mod file_types;

/// Manifest and lockfile parsers for each supported ecosystem.
pub mod parsers;

/// LSP feature providers: diagnostics, inlay hints, code actions,
/// completion, and document links.
pub mod providers;

/// HTTP clients for fetching package metadata from crates.io, npm,
/// PyPI, Go Proxy, Packagist, pub.dev, NuGet, Maven Central, and RubyGems.
pub mod registries;

/// JSON and Markdown vulnerability report generation.
pub mod reports;

/// `WorkspaceEdit` builder for adding packages to the `.zed/settings.json`
/// ignore list.
pub mod settings_edit;

/// Shared string utilities: truncation with ellipsis and HTML escaping.
pub mod utils;

/// OSV-backed vulnerability scanning and per-package result caching.
pub mod vulnerabilities;
