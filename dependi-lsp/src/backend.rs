use core::fmt;
use std::time::Duration;
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use dashmap::DashMap;
use hashbrown::HashMap;
use reqwest::Client as HttpClient;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::cache::{HybridCache, ReadCache, WriteCache};
use crate::config::Config;
use crate::document::DocumentState;
use crate::file_types::FileType;
use crate::parsers::Parser;
use crate::parsers::cargo::CargoParser;
use crate::parsers::csharp::CsharpParser;
use crate::parsers::dart::DartParser;
use crate::parsers::go::GoParser;
use crate::parsers::maven::MavenParser;
use crate::parsers::npm::NpmParser;
use crate::parsers::php::PhpParser;
use crate::parsers::python::PythonParser;
use crate::parsers::ruby::RubyParser;
use crate::providers::code_actions::create_code_actions;
use crate::providers::completion::{fmt_release_age, get_completions};
use crate::providers::diagnostics::create_diagnostics;
use crate::providers::document_links::create_document_links;
use crate::providers::inlay_hints::create_inlay_hint;
use crate::registries::cargo_sparse::CargoSparseRegistry;
use crate::registries::crates_io::CratesIoRegistry;
use crate::registries::go_proxy::GoProxyRegistry;
use crate::registries::http_client::create_shared_client;
use crate::registries::maven_central::MavenCentralRegistry;
use crate::registries::npm::NpmRegistry;
use crate::registries::nuget::NuGetRegistry;
use crate::registries::packagist::PackagistRegistry;
use crate::registries::pub_dev::PubDevRegistry;
use crate::registries::pypi::PyPiRegistry;
use crate::registries::rubygems::RubyGemsRegistry;
use crate::registries::{Registry, VersionInfo, VulnerabilitySeverity};
use crate::reports::{VulnerabilityReportEntry, VulnerabilitySummary};
use crate::vulnerabilities::cache::VulnerabilityCache;
use crate::vulnerabilities::osv::OsvClient;
use crate::vulnerabilities::{VulnerabilityQuery, normalize_version_for_osv};
use crate::{
    auth::{EnvTokenProvider, TokenProviderManager, cargo_credentials, fmt_redact_token},
    reports::fmt_markdown_report,
};

/// Extract the `[package].name` field from a Cargo.toml manifest.
/// Used to pass the root package name to `parse_cargo_lock` for multi-version disambiguation.
fn cargo_root_package_name(manifest_content: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(manifest_content).ok()?;
    value
        .get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

/// Compute cache key for a dependency, including registry for Cargo alternative registries.
///
/// For Cargo deps with `registry = "name"`, the key is `crates:{registry}:{name}` to avoid
/// collisions between crates.io and private registries. All other deps use the standard key.
fn dep_cache_key(dep: &crate::parsers::Dependency, file_type: FileType) -> String {
    let dep_name = &*dep.name;
    match (dep.registry.as_deref(), file_type) {
        (Some(registry), FileType::Cargo) => format!("crates:{registry}:{dep_name}"),
        _ => file_type.cache_key(dep_name),
    }
}

/// Holds cloneable references to backend state for async document processing.
/// Used by debounce tasks to process documents after the debounce period.
#[derive(Clone)]
struct ProcessingContext {
    client: Client,
    config: Arc<RwLock<Config>>,
    documents: Arc<DashMap<Url, DocumentState>>,
    version_cache: Arc<HybridCache>,
    cargo_parser: Arc<CargoParser>,
    npm_parser: Arc<NpmParser>,
    python_parser: Arc<PythonParser>,
    go_parser: Arc<GoParser>,
    php_parser: Arc<PhpParser>,
    dart_parser: Arc<DartParser>,
    csharp_parser: Arc<CsharpParser>,
    ruby_parser: Arc<RubyParser>,
    maven_parser: Arc<MavenParser>,
    crates_io: Arc<CratesIoRegistry>,
    cargo_custom_registries: Arc<DashMap<String, Arc<CargoSparseRegistry>>>,
    npm_registry: Arc<tokio::sync::RwLock<NpmRegistry>>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    maven_central: Arc<tokio::sync::RwLock<MavenCentralRegistry>>,
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
    /// Per-(ecosystem, name, version) transitive vuln data shared across document processing runs.
    transitive_vuln_data: Arc<
        DashMap<crate::vulnerabilities::cache::VulnCacheKey, Vec<crate::registries::Vulnerability>>,
    >,
}

impl ProcessingContext {
    fn parse_document(&self, uri: &Url, content: &str) -> Vec<crate::parsers::Dependency> {
        match FileType::detect(uri) {
            Some(FileType::Cargo) => self.cargo_parser.parse(content),
            Some(FileType::Npm) => self.npm_parser.parse(content),
            Some(FileType::Python) => self.python_parser.parse(content),
            Some(FileType::Go) => self.go_parser.parse(content),
            Some(FileType::Php) => self.php_parser.parse(content),
            Some(FileType::Dart) => self.dart_parser.parse(content),
            Some(FileType::Csharp) => self.csharp_parser.parse(content),
            Some(FileType::Ruby) => self.ruby_parser.parse(content),
            Some(FileType::Maven) => self.maven_parser.parse(content),
            None => vec![],
        }
    }

    async fn process_document(&self, uri: &Url, content: &str) {
        let Some(file_type) = FileType::detect(uri) else {
            return;
        };

        let mut dependencies = self.parse_document(uri, content);

        // lockfile_graph is populated by the ecosystem-specific blocks below.
        let mut lockfile_graph: Option<
            std::sync::Arc<crate::parsers::lockfile_graph::LockfileGraph>,
        > = None;

        // Resolve versions from Cargo.lock for Cargo dependencies
        if file_type == FileType::Cargo
            && let Ok(cargo_toml_path) = uri.to_file_path()
            && let Some(lock_path) =
                crate::parsers::cargo_lock::find_cargo_lock(&cargo_toml_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    // Use parse_cargo_lock (HashMap) for resolution: it correctly
                    // disambiguates multi-version crates via the root package's dep list.
                    // parse_cargo_lock_graph is kept for the transitive walk below.
                    let root_name = cargo_root_package_name(content);
                    let version_map = crate::parsers::cargo_lock::parse_cargo_lock(
                        &lock_content,
                        root_name.as_deref(),
                    );
                    for dep in &mut dependencies {
                        if let Some(v) = version_map.get(&dep.name) {
                            dep.resolved_version = Some(v.clone());
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
                    let graph = crate::parsers::cargo_lock::parse_cargo_lock_graph(&lock_content);
                    lockfile_graph = Some(std::sync::Arc::new(graph));
                }
                Err(e) => {
                    tracing::debug!(
                        "Could not read Cargo.lock at {}: {}",
                        lock_path.display(),
                        e
                    );
                }
            }
        }

        // Resolve versions from lockfile for npm dependencies
        if file_type == FileType::Npm
            && let Ok(package_json_path) = uri.to_file_path()
            && let Some((lock_path, npm_lockfile_type)) =
                crate::parsers::npm_lock::find_npm_lockfile(&package_json_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    use crate::parsers::npm_lock::NpmLockfileType;
                    let graph = match npm_lockfile_type {
                        NpmLockfileType::PackageLock => {
                            crate::parsers::npm_lock::parse_package_lock_graph(&lock_content)
                        }
                        NpmLockfileType::PnpmLock => {
                            crate::parsers::npm_lock::parse_pnpm_lock_graph(&lock_content)
                        }
                        NpmLockfileType::YarnLock => {
                            crate::parsers::npm_lock::parse_yarn_lock_graph(&lock_content)
                        }
                        NpmLockfileType::BunLock => {
                            // No graph parser for Bun — fall back to HashMap path
                            let lock_versions = crate::parsers::npm_lock::parse_npm_lockfile(
                                &lock_content,
                                npm_lockfile_type,
                            );
                            crate::parsers::lockfile_graph::LockfileGraph {
                                packages: lock_versions
                                    .iter()
                                    .map(|(name, version)| {
                                        crate::parsers::lockfile_graph::LockfilePackage {
                                            name: name.clone(),
                                            version: version.clone(),
                                            dependencies: Vec::new(),
                                            is_root: false,
                                        }
                                    })
                                    .collect(),
                            }
                        }
                    };
                    // Build first-wins version map: for ecosystems without canonical
                    // multi-version disambiguation, the first entry for a given name wins.
                    // This matches the pre-graph HashMap-based behavior for these ecosystems.
                    let mut version_map: hashbrown::HashMap<String, String> =
                        hashbrown::HashMap::new();
                    for p in &graph.packages {
                        version_map
                            .entry_ref(&p.name)
                            .or_insert_with(|| p.version.clone());
                    }
                    for dep in &mut dependencies {
                        if let Some(v) = version_map.get(&dep.name) {
                            dep.resolved_version = Some(v.clone());
                        }
                    }
                    tracing::debug!(
                        "Resolved {} versions from {} ({:?})",
                        dependencies
                            .iter()
                            .filter(|d| d.resolved_version.is_some())
                            .count(),
                        lock_path.display(),
                        npm_lockfile_type,
                    );
                    lockfile_graph = Some(std::sync::Arc::new(graph));
                }
                Err(e) => {
                    tracing::debug!("Could not read lockfile at {}: {}", lock_path.display(), e);
                }
            }
        }

        // Resolve versions from lockfile for Python dependencies
        if file_type == FileType::Python
            && let Ok(manifest_path) = uri.to_file_path()
        {
            let preferred = crate::parsers::python_lock::detect_python_tool(content);
            if let Some((lock_path, py_lockfile_type)) =
                crate::parsers::python_lock::find_python_lockfile(&manifest_path, preferred).await
            {
                match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                    Ok(lock_content) => {
                        use crate::parsers::python_lock::PythonLockfileType;
                        let graph = match py_lockfile_type {
                            PythonLockfileType::PoetryLock => {
                                crate::parsers::python_lock::parse_poetry_lock_graph(&lock_content)
                            }
                            PythonLockfileType::UvLock => {
                                crate::parsers::python_lock::parse_uv_lock_graph(&lock_content)
                            }
                            PythonLockfileType::PipfileLock => {
                                crate::parsers::python_lock::parse_pipfile_lock_graph(&lock_content)
                            }
                            PythonLockfileType::PdmLock => {
                                // No graph parser for PDM — build minimal graph from HashMap
                                let lock_versions =
                                    crate::parsers::python_lock::parse_python_lockfile(
                                        &lock_content,
                                        py_lockfile_type,
                                    );
                                crate::parsers::lockfile_graph::LockfileGraph {
                                    packages: lock_versions
                                        .iter()
                                        .map(|(name, version)| {
                                            crate::parsers::lockfile_graph::LockfilePackage {
                                                name: name.clone(),
                                                version: version.clone(),
                                                dependencies: Vec::new(),
                                                is_root: false,
                                            }
                                        })
                                        .collect(),
                                }
                            }
                        };
                        // Build first-wins version map (graph keys are already normalized).
                        // For ecosystems without canonical multi-version disambiguation, the first
                        // entry for a given name wins — matches pre-graph HashMap behavior.
                        let mut version_map: hashbrown::HashMap<String, String> =
                            hashbrown::HashMap::new();
                        for p in &graph.packages {
                            version_map
                                .entry_ref(&p.name)
                                .or_insert_with(|| p.version.clone());
                        }
                        for dep in &mut dependencies {
                            let normalized =
                                crate::parsers::python_lock::normalize_python_name(&dep.name);
                            if let Some(v) = version_map.get(&normalized) {
                                dep.resolved_version = Some(v.clone());
                            }
                        }
                        tracing::debug!(
                            "Resolved {} versions from {} ({:?})",
                            dependencies
                                .iter()
                                .filter(|d| d.resolved_version.is_some())
                                .count(),
                            lock_path.display(),
                            py_lockfile_type,
                        );
                        lockfile_graph = Some(std::sync::Arc::new(graph));
                    }
                    Err(e) => {
                        tracing::debug!(
                            "Could not read lockfile at {}: {}",
                            lock_path.display(),
                            e
                        );
                    }
                }
            }
        }

        // Resolve versions from go.sum for Go dependencies
        if file_type == FileType::Go
            && let Ok(go_mod_path) = uri.to_file_path()
            && let Some(lock_path) = crate::parsers::go_sum::find_go_sum(&go_mod_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    let lock_versions = crate::parsers::go_sum::parse_go_sum(&lock_content);
                    let mut minimal_packages = Vec::new();
                    for dep in &mut dependencies {
                        if let Some(versions) = lock_versions.get(&dep.name) {
                            // Prefer dep.version when it appears among the
                            // candidates (confirms go.mod and go.sum agree).
                            // Fall back to auto-select only when exactly one
                            // candidate exists; skip ambiguous multi-version
                            // entries to avoid guessing.
                            if versions.iter().any(|v| v == &dep.version) {
                                dep.resolved_version = Some(dep.version.clone());
                            } else if versions.len() == 1 {
                                dep.resolved_version = Some(versions[0].clone());
                            }
                        }
                    }
                    // Build minimal graph (no edge data available from go.sum)
                    for (name, versions) in &lock_versions {
                        for version in versions {
                            minimal_packages.push(
                                crate::parsers::lockfile_graph::LockfilePackage {
                                    name: name.clone(),
                                    version: version.clone(),
                                    dependencies: Vec::new(),
                                    is_root: false,
                                },
                            );
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
                    lockfile_graph = Some(std::sync::Arc::new(
                        crate::parsers::lockfile_graph::LockfileGraph {
                            packages: minimal_packages,
                        },
                    ));
                }
                Err(e) => {
                    tracing::debug!("Could not read go.sum at {}: {}", lock_path.display(), e);
                }
            }
        }

        // Resolve versions from composer.lock for PHP dependencies
        if file_type == FileType::Php
            && let Ok(composer_json_path) = uri.to_file_path()
            && let Some(lock_path) =
                crate::parsers::composer_lock::find_composer_lock(&composer_json_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    let graph =
                        crate::parsers::composer_lock::parse_composer_lock_graph(&lock_content);
                    // Build first-wins version map (graph keys are already normalized).
                    // For ecosystems without canonical multi-version disambiguation, the first
                    // entry for a given name wins — matches pre-graph HashMap behavior.
                    let mut version_map: hashbrown::HashMap<String, String> =
                        hashbrown::HashMap::new();
                    for p in &graph.packages {
                        version_map
                            .entry_ref(&p.name)
                            .or_insert_with(|| p.version.clone());
                    }
                    for dep in &mut dependencies {
                        let normalized =
                            crate::parsers::composer_lock::normalize_composer_name(&dep.name);
                        if let Some(v) = version_map.get(&normalized) {
                            dep.resolved_version = Some(v.clone());
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
                    lockfile_graph = Some(std::sync::Arc::new(graph));
                }
                Err(e) => {
                    tracing::debug!(
                        "Could not read composer.lock at {}: {}",
                        lock_path.display(),
                        e
                    );
                }
            }
        }

        // Resolve versions from pubspec.lock for Dart dependencies
        if file_type == FileType::Dart
            && let Ok(pubspec_yaml_path) = uri.to_file_path()
            && let Some(lock_path) =
                crate::parsers::pubspec_lock::find_pubspec_lock(&pubspec_yaml_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    let lock_versions =
                        crate::parsers::pubspec_lock::parse_pubspec_lock(&lock_content);
                    for dep in &mut dependencies {
                        if let Some(resolved) = lock_versions.get(&dep.name) {
                            dep.resolved_version = Some(resolved.clone());
                        }
                    }
                    tracing::debug!(
                        "Resolved {} Dart versions from pubspec.lock at {}",
                        dependencies
                            .iter()
                            .filter(|d| d.resolved_version.is_some())
                            .count(),
                        lock_path.display()
                    );
                    // No graph parser for Dart — build minimal graph from HashMap
                    lockfile_graph = Some(std::sync::Arc::new(
                        crate::parsers::lockfile_graph::LockfileGraph {
                            packages: lock_versions
                                .into_iter()
                                .map(|(name, version)| {
                                    crate::parsers::lockfile_graph::LockfilePackage {
                                        name,
                                        version,
                                        dependencies: Vec::new(),
                                        is_root: false,
                                    }
                                })
                                .collect(),
                        },
                    ));
                }
                Err(e) => {
                    tracing::debug!(
                        "Could not read pubspec.lock at {}: {e}",
                        lock_path.display(),
                    );
                }
            }
        }

        // Resolve versions from packages.lock.json for C# dependencies
        if file_type == FileType::Csharp
            && let Ok(csproj_path) = uri.to_file_path()
            && let Some(lock_path) =
                crate::parsers::packages_lock_json::find_packages_lock(&csproj_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    let lock_versions =
                        crate::parsers::packages_lock_json::parse_packages_lock(&lock_content);
                    for dep in &mut dependencies {
                        let normalized =
                            crate::parsers::packages_lock_json::normalize_nuget_name(&dep.name);
                        if let Some(resolved) = lock_versions.get(&normalized) {
                            dep.resolved_version = Some(resolved.clone());
                        }
                    }
                    tracing::debug!(
                        "Resolved {} C# versions from packages.lock.json at {}",
                        dependencies
                            .iter()
                            .filter(|d| d.resolved_version.is_some())
                            .count(),
                        lock_path.display()
                    );
                    // No graph parser for C# — build minimal graph from HashMap
                    lockfile_graph = Some(std::sync::Arc::new(
                        crate::parsers::lockfile_graph::LockfileGraph {
                            packages: lock_versions
                                .into_iter()
                                .map(|(name, version)| {
                                    crate::parsers::lockfile_graph::LockfilePackage {
                                        name,
                                        version,
                                        dependencies: Vec::new(),
                                        is_root: false,
                                    }
                                })
                                .collect(),
                        },
                    ));
                }
                Err(e) => {
                    tracing::debug!(
                        "Could not read packages.lock.json at {}: {e}",
                        lock_path.display(),
                    );
                }
            }
        }

        // Resolve versions from Gemfile.lock for Ruby dependencies
        if file_type == FileType::Ruby
            && let Ok(gemfile_path) = uri.to_file_path()
            && let Some(lock_path) =
                crate::parsers::gemfile_lock::find_gemfile_lock(&gemfile_path).await
        {
            match crate::parsers::lockfile_graph::read_lockfile_capped(&lock_path).await {
                Ok(lock_content) => {
                    let graph =
                        crate::parsers::gemfile_lock::parse_gemfile_lock_graph(&lock_content);
                    // Build first-wins version map (graph keys are already normalized).
                    // For ecosystems without canonical multi-version disambiguation, the first
                    // entry for a given name wins — matches pre-graph HashMap behavior.
                    let mut version_map: hashbrown::HashMap<String, String> =
                        hashbrown::HashMap::new();
                    for p in &graph.packages {
                        version_map
                            .entry_ref(&p.name)
                            .or_insert_with(|| p.version.clone());
                    }
                    for dep in &mut dependencies {
                        let normalized =
                            crate::parsers::gemfile_lock::normalize_gem_name(&dep.name);
                        if let Some(v) = version_map.get(&normalized) {
                            dep.resolved_version = Some(v.clone());
                        }
                    }
                    tracing::debug!(
                        "Resolved {} Ruby versions from Gemfile.lock at {}",
                        dependencies
                            .iter()
                            .filter(|d| d.resolved_version.is_some())
                            .count(),
                        lock_path.display()
                    );
                    lockfile_graph = Some(std::sync::Arc::new(graph));
                }
                Err(e) => {
                    tracing::debug!(
                        "Could not read Gemfile.lock at {}: {e}",
                        lock_path.display(),
                    );
                }
            }
        }

        tracing::info!(
            "Parsed {} dependencies from {}",
            dependencies.len(),
            uri.path()
        );

        // Clone Arc references for async tasks
        let crates_io = Arc::clone(&self.crates_io);
        let cargo_custom_registries = Arc::clone(&self.cargo_custom_registries);
        let npm_registry = Arc::clone(&self.npm_registry);
        let pypi = Arc::clone(&self.pypi);
        let go_proxy = Arc::clone(&self.go_proxy);
        let packagist = Arc::clone(&self.packagist);
        let pub_dev = Arc::clone(&self.pub_dev);
        let nuget = Arc::clone(&self.nuget);
        let rubygems = Arc::clone(&self.rubygems);
        let maven_central = Arc::clone(&self.maven_central);
        let cache = Arc::clone(&self.version_cache);

        let fetch_tasks: Vec<_> = dependencies
            .iter()
            .map(|dep| {
                let name = dep.name.clone();
                let registry = dep.registry.clone();
                let cache_key = dep_cache_key(dep, file_type);
                let crates_io = Arc::clone(&crates_io);
                let cargo_custom_registries = Arc::clone(&cargo_custom_registries);
                let npm_registry = Arc::clone(&npm_registry);
                let pypi = Arc::clone(&pypi);
                let go_proxy = Arc::clone(&go_proxy);
                let packagist = Arc::clone(&packagist);
                let pub_dev = Arc::clone(&pub_dev);
                let nuget = Arc::clone(&nuget);
                let rubygems = Arc::clone(&rubygems);
                let maven_central = Arc::clone(&maven_central);
                let cache = Arc::clone(&cache);
                async move {
                    // Check cache first
                    if cache.get(&cache_key).is_some() {
                        tracing::debug!("Cache hit for '{name}' (key: {cache_key})");
                        return;
                    }
                    tracing::debug!("Cache miss for '{name}' (key: {cache_key}), fetching from registry {registry:?}");
                    // Fetch from appropriate registry
                    let result = match file_type {
                        FileType::Cargo => {
                            if let Some(ref reg_name) = registry {
                                if let Some(reg) = cargo_custom_registries.get(reg_name) {
                                    reg.get_version_info(&name).await
                                } else {
                                    tracing::warn!(
                                        "Unknown Cargo registry '{reg_name}' for package '{name}', falling back to crates.io",
                                    );
                                    crates_io.get_version_info(&name).await
                                }
                            } else {
                                crates_io.get_version_info(&name).await
                            }
                        }
                        FileType::Npm => npm_registry.read().await.get_version_info(&name).await,
                        FileType::Python => pypi.get_version_info(&name).await,
                        FileType::Go => go_proxy.get_version_info(&name).await,
                        FileType::Php => packagist.get_version_info(&name).await,
                        FileType::Dart => pub_dev.get_version_info(&name).await,
                        FileType::Csharp => nuget.get_version_info(&name).await,
                        FileType::Ruby => rubygems.get_version_info(&name).await,
                        FileType::Maven => {
                            maven_central.read().await.get_version_info(&name).await
                        }
                    };
                    match result {
                        Ok(info) => {
                            cache.insert(cache_key, info);
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to fetch version info for '{name}' (registry: {registry:?}): {e}",
                            );
                        }
                    }
                }
            })
            .collect();

        // Run up to 5 concurrent requests
        let semaphore = Arc::new(tokio::sync::Semaphore::new(5));
        let handles: Vec<_> = fetch_tasks
            .into_iter()
            .map(|task| {
                let permit = Arc::clone(&semaphore);
                tokio::spawn(async move {
                    let _permit = permit.acquire().await;
                    task.await
                })
            })
            .collect();

        // Wait for all tasks to complete
        for handle in handles {
            let _ = handle.await;
        }

        // Store document state IMMEDIATELY (before vulnerability check)
        self.documents.insert(
            uri.clone(),
            DocumentState {
                dependencies: dependencies.clone(),
                file_type,
                lockfile_graph: lockfile_graph.clone(),
                transitive_vulns_by_direct: hashbrown::HashMap::new(),
            },
        );

        // Publish diagnostics IMMEDIATELY (versions are available, vulnerabilities will update later)
        let (diagnostics_enabled, security_show_diags, min_severity, security_enabled) = self
            .config
            .read()
            .map(|c| {
                (
                    c.diagnostics.enabled,
                    c.security.show_diagnostics,
                    if c.security.show_diagnostics {
                        Some(c.security.min_severity_level())
                    } else {
                        None
                    },
                    c.security.enabled,
                )
            })
            .unwrap_or((true, true, None, true));

        if diagnostics_enabled {
            let severity_filter = if security_show_diags {
                min_severity
            } else {
                None
            };
            // Pre-build cache key map for registry-aware lookups
            let cache_key_map: HashMap<String, String> = dependencies
                .iter()
                .map(|dep| (dep.name.clone(), dep_cache_key(dep, file_type)))
                .collect();
            // Transitive vulns are not yet available at this point (background task hasn't run).
            // They will be populated in DocumentState.transitive_vulns_by_direct once the
            // background vulnerability fetch completes.
            let empty_transitives: hashbrown::HashMap<
                String,
                Vec<crate::registries::TransitiveVuln>,
            > = hashbrown::HashMap::new();
            let diagnostics = create_diagnostics(
                &dependencies,
                &self.version_cache,
                |name| {
                    cache_key_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| file_type.cache_key(name))
                },
                severity_filter,
                file_type,
                &empty_transitives,
            );

            self.client
                .publish_diagnostics(uri.clone(), diagnostics, None)
                .await;
        }

        // Refresh inlay hints IMMEDIATELY (versions are available)
        self.client
            .send_request::<request::InlayHintRefreshRequest>(())
            .await
            .ok();

        // Fetch vulnerabilities from OSV.dev in BACKGROUND (non-blocking)
        if security_enabled && !dependencies.is_empty() {
            let dependencies_clone = dependencies.clone();
            let cache_clone = Arc::clone(&self.version_cache);
            let osv_client_clone = Arc::clone(&self.osv_client);
            let vuln_cache_clone = Arc::clone(&self.vuln_cache);
            let transitive_vuln_data_clone = Arc::clone(&self.transitive_vuln_data);
            let client_clone = self.client.clone();
            let documents_clone = Arc::clone(&self.documents);
            let uri_clone = uri.clone();

            tokio::spawn(async move {
                DependiBackend::fetch_vulnerabilities_background(
                    dependencies_clone,
                    file_type,
                    cache_clone,
                    osv_client_clone,
                    vuln_cache_clone,
                    client_clone,
                    VulnBgContext {
                        documents: documents_clone,
                        uri: uri_clone,
                        lockfile_graph,
                        transitive_vuln_data: transitive_vuln_data_clone,
                    },
                )
                .await;
            });
        }
    }
}

/// Context passed to the background vulnerability fetch task so it can write
/// per-document transitive attribution after the OSV query completes.
struct VulnBgContext {
    documents: Arc<DashMap<Url, DocumentState>>,
    uri: Url,
    lockfile_graph: Option<std::sync::Arc<crate::parsers::lockfile_graph::LockfileGraph>>,
    /// Per-(ecosystem, name, version) transitive vuln data. Populated by the fresh-query loop
    /// and read by the cached-query loop so re-attribution works on subsequent document opens.
    transitive_vuln_data: Arc<
        DashMap<crate::vulnerabilities::cache::VulnCacheKey, Vec<crate::registries::Vulnerability>>,
    >,
}

pub struct DependiBackend {
    client: Client,
    /// Configuration (Arc-wrapped for sharing with debounce tasks)
    config: Arc<RwLock<Config>>,
    /// Cache for documents and their parsed state (Arc-wrapped for sharing with debounce tasks)
    documents: Arc<DashMap<Url, DocumentState>>,
    /// Cache for version information (keyed by "registry:package")
    version_cache: Arc<HybridCache>,
    /// Parsers (Arc-wrapped for sharing with debounce tasks)
    cargo_parser: Arc<CargoParser>,
    npm_parser: Arc<NpmParser>,
    python_parser: Arc<PythonParser>,
    go_parser: Arc<GoParser>,
    php_parser: Arc<PhpParser>,
    dart_parser: Arc<DartParser>,
    csharp_parser: Arc<CsharpParser>,
    ruby_parser: Arc<RubyParser>,
    maven_parser: Arc<MavenParser>,
    /// Registry clients
    crates_io: Arc<CratesIoRegistry>,
    /// Cargo alternative registries (registry name -> sparse registry client)
    cargo_custom_registries: Arc<DashMap<String, Arc<CargoSparseRegistry>>>,
    /// npm registry (tokio::sync::RwLock-wrapped to allow reconfiguration during initialize)
    npm_registry: Arc<tokio::sync::RwLock<NpmRegistry>>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    maven_central: Arc<tokio::sync::RwLock<MavenCentralRegistry>>,
    /// Shared HTTP client for creating new registry instances
    http_client: Arc<HttpClient>,
    /// Token provider manager for authentication across all ecosystems
    token_manager: Arc<TokenProviderManager>,
    /// Vulnerability scanning
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
    /// Per-(ecosystem, name, version) transitive vuln data.
    /// Populated during fresh OSV queries for transitive packages; read on cached re-attribution
    /// so subsequent document opens can still attribute transitives to their direct parents.
    transitive_vuln_data: Arc<
        DashMap<crate::vulnerabilities::cache::VulnCacheKey, Vec<crate::registries::Vulnerability>>,
    >,
    /// Debounce tasks for did_change notifications (per-URI)
    /// Maps URI -> (generation, JoinHandle) for safe cleanup with racing tasks
    debounce_tasks: Arc<DashMap<Url, (u64, tokio::task::JoinHandle<()>)>>,
    /// Generation counter for debounce tasks (incremented on each spawn)
    debounce_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Pending content changes awaiting debounce completion
    pending_changes: Arc<DashMap<Url, String>>,
}

impl DependiBackend {
    /// Create a new DependiBackend with default configuration, parsers, registry clients,
    /// caches, and an OSV client.
    ///
    /// The provided `client` is used for LSP communication. A shared HTTP client is created
    /// internally and used to construct registry clients bound to that HTTP client.
    ///
    /// # Examples
    ///
    /// ```
    /// // Obtain an LSP `Client` from the runtime environment and pass it in:
    /// // let client = /* LSP client */ ;
    /// // let backend = DependiBackend::new(client);
    /// ```
    pub fn new(client: Client) -> Self {
        Self::with_http_client(client, None)
    }

    pub fn with_http_client(client: Client, http_client: Option<Arc<HttpClient>>) -> Self {
        let http_client = http_client.unwrap_or_else(|| {
            create_shared_client().expect("Failed to create shared HTTP client")
        });

        let config = Config::default();
        let npm_registry = Arc::new(tokio::sync::RwLock::new(
            NpmRegistry::with_client_and_config(Arc::clone(&http_client), &config.registries.npm),
        ));
        let maven_central = Arc::new(tokio::sync::RwLock::new(
            MavenCentralRegistry::with_client_and_config(
                Arc::clone(&http_client),
                &config.registries.maven,
            ),
        ));

        // Create token provider manager for centralized auth management
        let token_manager = Arc::new(TokenProviderManager::new());

        Self {
            client,
            config: Arc::new(RwLock::new(config)),
            documents: Arc::new(DashMap::new()),
            version_cache: Arc::new(HybridCache::new()),
            cargo_parser: Arc::new(CargoParser::new()),
            npm_parser: Arc::new(NpmParser::new()),
            python_parser: Arc::new(PythonParser::new()),
            go_parser: Arc::new(GoParser::new()),
            php_parser: Arc::new(PhpParser::new()),
            dart_parser: Arc::new(DartParser::new()),
            csharp_parser: Arc::new(CsharpParser::new()),
            ruby_parser: Arc::new(RubyParser::new()),
            maven_parser: Arc::new(MavenParser::new()),
            crates_io: Arc::new(CratesIoRegistry::with_client(Arc::clone(&http_client))),
            cargo_custom_registries: Arc::new(DashMap::new()),
            npm_registry,
            pypi: Arc::new(PyPiRegistry::with_client(Arc::clone(&http_client))),
            go_proxy: Arc::new(GoProxyRegistry::with_client(Arc::clone(&http_client))),
            packagist: Arc::new(PackagistRegistry::with_client(Arc::clone(&http_client))),
            pub_dev: Arc::new(PubDevRegistry::with_client(Arc::clone(&http_client))),
            nuget: Arc::new(NuGetRegistry::with_client(Arc::clone(&http_client))),
            rubygems: Arc::new(RubyGemsRegistry::with_client(Arc::clone(&http_client))),
            maven_central,
            http_client,
            token_manager,
            osv_client: Arc::new(OsvClient::default()),
            vuln_cache: Arc::new(VulnerabilityCache::new()),
            transitive_vuln_data: Arc::new(DashMap::new()),
            debounce_tasks: Arc::new(DashMap::new()),
            debounce_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pending_changes: Arc::new(DashMap::new()),
        }
    }

    fn create_processing_context(&self) -> ProcessingContext {
        ProcessingContext {
            client: self.client.clone(),
            config: Arc::clone(&self.config),
            documents: Arc::clone(&self.documents),
            version_cache: Arc::clone(&self.version_cache),
            cargo_parser: Arc::clone(&self.cargo_parser),
            npm_parser: Arc::clone(&self.npm_parser),
            python_parser: Arc::clone(&self.python_parser),
            go_parser: Arc::clone(&self.go_parser),
            php_parser: Arc::clone(&self.php_parser),
            dart_parser: Arc::clone(&self.dart_parser),
            csharp_parser: Arc::clone(&self.csharp_parser),
            ruby_parser: Arc::clone(&self.ruby_parser),
            maven_parser: Arc::clone(&self.maven_parser),
            crates_io: Arc::clone(&self.crates_io),
            cargo_custom_registries: Arc::clone(&self.cargo_custom_registries),
            npm_registry: Arc::clone(&self.npm_registry),
            pypi: Arc::clone(&self.pypi),
            go_proxy: Arc::clone(&self.go_proxy),
            packagist: Arc::clone(&self.packagist),
            pub_dev: Arc::clone(&self.pub_dev),
            nuget: Arc::clone(&self.nuget),
            rubygems: Arc::clone(&self.rubygems),
            maven_central: Arc::clone(&self.maven_central),
            osv_client: Arc::clone(&self.osv_client),
            vuln_cache: Arc::clone(&self.vuln_cache),
            transitive_vuln_data: Arc::clone(&self.transitive_vuln_data),
        }
    }

    async fn get_version_info(
        &self,
        file_type: FileType,
        package_name: &str,
        cargo_registry: Option<&str>,
    ) -> Option<VersionInfo> {
        let cache_key = match (cargo_registry, file_type) {
            (Some(registry), FileType::Cargo) => {
                format!("crates:{registry}:{package_name}")
            }
            _ => file_type.cache_key(package_name),
        };

        // Check cache first
        if let Some(cached) = self.version_cache.get(&cache_key) {
            return Some(cached);
        }

        // Fetch from appropriate registry
        let result = match file_type {
            FileType::Cargo => {
                if let Some(reg_name) = cargo_registry {
                    if let Some(reg) = self.cargo_custom_registries.get(reg_name) {
                        reg.get_version_info(package_name).await
                    } else {
                        self.crates_io.get_version_info(package_name).await
                    }
                } else {
                    self.crates_io.get_version_info(package_name).await
                }
            }
            FileType::Npm => {
                self.npm_registry
                    .read()
                    .await
                    .get_version_info(package_name)
                    .await
            }
            FileType::Python => self.pypi.get_version_info(package_name).await,
            FileType::Go => self.go_proxy.get_version_info(package_name).await,
            FileType::Php => self.packagist.get_version_info(package_name).await,
            FileType::Dart => self.pub_dev.get_version_info(package_name).await,
            FileType::Csharp => self.nuget.get_version_info(package_name).await,
            FileType::Ruby => self.rubygems.get_version_info(package_name).await,
            FileType::Maven => {
                self.maven_central
                    .read()
                    .await
                    .get_version_info(package_name)
                    .await
            }
        };

        match result {
            Ok(info) => {
                self.version_cache.insert(cache_key, info.clone());
                Some(info)
            }
            Err(e) => {
                tracing::warn!("Failed to fetch version info for {}: {}", package_name, e);
                None
            }
        }
    }

    async fn fetch_vulnerabilities_background(
        dependencies: Vec<crate::parsers::Dependency>,
        file_type: FileType,
        cache: Arc<HybridCache>,
        osv_client: Arc<OsvClient>,
        vuln_cache: Arc<VulnerabilityCache>,
        client: Client,
        bg_ctx: VulnBgContext,
    ) {
        use crate::vulnerabilities::cache::VulnCacheKey;

        let ecosystem = file_type.to_ecosystem();

        // Pre-build cache key map for registry-aware lookups
        let cache_key_map: HashMap<String, String> = dependencies
            .iter()
            .map(|dep| (dep.name.clone(), dep_cache_key(dep, file_type)))
            .collect();

        // Collect transitive packages from the lockfile graph.
        // Names are canonicalized to match the lockfile graph keys (Python/PHP/Ruby normalize).
        // Also build a reverse map so that normalized parent names from reverse_index can be
        // translated back to the raw manifest name used as keys in diagnostics/hover lookups.
        let (direct_names, normalized_to_raw): (Vec<String>, HashMap<String, String>) = {
            let mut names = Vec::with_capacity(dependencies.len());
            let mut map: HashMap<String, String> = HashMap::new();
            for d in dependencies.iter() {
                let canonical = match file_type {
                    FileType::Python => crate::parsers::python_lock::normalize_python_name(&d.name),
                    FileType::Php => {
                        crate::parsers::composer_lock::normalize_composer_name(&d.name)
                    }
                    FileType::Ruby => crate::parsers::gemfile_lock::normalize_gem_name(&d.name),
                    _ => d.name.clone(),
                };
                names.push(canonical.clone());
                map.insert(canonical, d.name.clone());
            }
            (names, map)
        };
        let transitives: Vec<crate::parsers::lockfile_graph::LockfilePackage> = bg_ctx
            .lockfile_graph
            .as_deref()
            .map(|g| {
                g.transitives_only(&direct_names)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // Build queries for direct packages not in vulnerability cache.
        // Track the filtered subset so we can correlate results back.
        let mut direct_queries: Vec<VulnerabilityQuery> = Vec::new();
        let mut direct_query_deps: Vec<&crate::parsers::Dependency> = Vec::new();
        for dep in dependencies.iter() {
            let normalized_version = normalize_version_for_osv(dep.effective_version());
            let vuln_key = VulnCacheKey::new(ecosystem, &dep.name, &normalized_version);
            if vuln_cache.contains(&vuln_key) {
                continue;
            }
            direct_queries.push(VulnerabilityQuery {
                ecosystem,
                package_name: dep.name.clone(),
                version: normalized_version,
            });
            direct_query_deps.push(dep);
        }

        // Build queries for transitive packages not in vulnerability cache.
        // Cached transitives are tracked separately so we can still attribute their vulns.
        let mut transitive_queries: Vec<VulnerabilityQuery> = Vec::new();
        let mut transitive_query_pkgs: Vec<&crate::parsers::lockfile_graph::LockfilePackage> =
            Vec::new();
        let mut transitive_cached_pkgs: Vec<&crate::parsers::lockfile_graph::LockfilePackage> =
            Vec::new();
        for t in transitives.iter() {
            let normalized_version = normalize_version_for_osv(&t.version);
            let vuln_key = VulnCacheKey::new(ecosystem, &t.name, &normalized_version);
            if vuln_cache.contains(&vuln_key) {
                transitive_cached_pkgs.push(t);
                continue;
            }
            transitive_queries.push(VulnerabilityQuery {
                ecosystem,
                package_name: t.name.clone(),
                version: normalized_version,
            });
            transitive_query_pkgs.push(t);
        }

        let direct_count = direct_queries.len();
        let mut all_queries = direct_queries;
        all_queries.extend(transitive_queries);

        tracing::info!(
            "Background: Querying OSV.dev for {} packages ({} direct, {} transitive)",
            all_queries.len(),
            direct_count,
            transitive_query_pkgs.len()
        );

        // Batch query OSV.dev
        match osv_client.query_batch(&all_queries).await {
            Ok(results) => {
                let mut updated_count = 0;

                let (direct_results, transitive_results) = results.split_at(direct_count);

                // Update vulnerability cache and version_cache with direct results.
                // direct_query_deps is the filtered list (cached ones were skipped above).
                for (dep, result) in direct_query_deps.iter().zip(direct_results.iter()) {
                    let normalized_version = normalize_version_for_osv(dep.effective_version());
                    // Mark this package as queried in vuln_cache
                    let vuln_key = VulnCacheKey::new(ecosystem, &dep.name, &normalized_version);
                    vuln_cache.insert(vuln_key);

                    // Store vulnerabilities and deprecated status in version_cache
                    let cache_key = cache_key_map
                        .get(&dep.name)
                        .cloned()
                        .unwrap_or_else(|| file_type.cache_key(&dep.name));
                    if let Some(mut info) = cache.get(&cache_key) {
                        info.vulnerabilities = result.vulnerabilities.clone();
                        info.deprecated = result.deprecated;
                        if result.deprecated {
                            tracing::info!(
                                "Background: Package {} {} is deprecated (unmaintained)",
                                dep.name,
                                normalized_version
                            );
                        }
                        tracing::debug!(
                            "Background: Updated {} {} with {} vulnerabilities, deprecated={}",
                            dep.name,
                            normalized_version,
                            result.vulnerabilities.len(),
                            result.deprecated
                        );
                        cache.insert(cache_key, info);
                        updated_count += 1;
                    } else {
                        tracing::warn!(
                            "Background: Could not update vulnerabilities for {}: not found in version cache",
                            dep.name
                        );
                    }
                }

                // Attribute transitive vulnerabilities to ALL direct parents that reach them.
                // Stored per-document (not in global version_cache) to avoid cross-workspace
                // contamination: transitive attribution depends on this document's lockfile graph.
                // transitive_query_pkgs is the filtered list (cached ones were skipped above).
                if let Some(graph) = bg_ctx.lockfile_graph.as_deref() {
                    use crate::registries::TransitiveVuln;

                    let inverse = graph.reverse_index(&direct_names);

                    // Build per-document transitive attribution map.
                    let mut transitive_vulns_by_direct: hashbrown::HashMap<
                        String,
                        Vec<TransitiveVuln>,
                    > = hashbrown::HashMap::new();

                    for (tpkg, result) in
                        transitive_query_pkgs.iter().zip(transitive_results.iter())
                    {
                        // Mark this transitive package as queried in vuln_cache
                        let normalized_version = normalize_version_for_osv(&tpkg.version);
                        let vuln_key =
                            VulnCacheKey::new(ecosystem, &tpkg.name, &normalized_version);
                        vuln_cache.insert(vuln_key);

                        let vuln_data_key = VulnCacheKey::new(
                            ecosystem,
                            &tpkg.name,
                            &normalize_version_for_osv(&tpkg.version),
                        );
                        if !result.vulnerabilities.is_empty() {
                            bg_ctx
                                .transitive_vuln_data
                                .insert(vuln_data_key, result.vulnerabilities.clone());
                        }

                        if result.vulnerabilities.is_empty() {
                            continue;
                        }

                        // Attribute to ALL direct parents that transitively reach this package.
                        let parents = inverse.get(&tpkg.name).cloned().unwrap_or_default();

                        if parents.is_empty() {
                            // No known parent — attribute to "(unknown)" so we don't drop the finding.
                            for v in &result.vulnerabilities {
                                transitive_vulns_by_direct
                                    .entry_ref("(unknown)")
                                    .or_default()
                                    .push(TransitiveVuln {
                                        package_name: tpkg.name.clone(),
                                        package_version: tpkg.version.clone(),
                                        vulnerability: v.clone(),
                                    });
                            }
                        } else {
                            for parent in &parents {
                                // Translate normalized parent name back to the raw manifest name
                                // so that diagnostics/hover lookups using dep.name succeed.
                                let raw_parent = normalized_to_raw
                                    .get(parent.as_str())
                                    .cloned()
                                    .unwrap_or_else(|| parent.clone());
                                for v in &result.vulnerabilities {
                                    transitive_vulns_by_direct
                                        .entry_ref(raw_parent.as_str())
                                        .or_default()
                                        .push(TransitiveVuln {
                                            package_name: tpkg.name.clone(),
                                            package_version: tpkg.version.clone(),
                                            vulnerability: v.clone(),
                                        });
                                }
                                tracing::debug!(
                                    "Background: Attributed {} transitive vulns from {}@{} to direct dep {}",
                                    result.vulnerabilities.len(),
                                    tpkg.name,
                                    tpkg.version,
                                    raw_parent
                                );
                            }
                        }
                    }

                    // Attribute transitive vulns for packages already in vuln_cache.
                    // Vuln data for transitives is stored in transitive_vuln_data (not
                    // version_cache, which only holds direct-dep data). This ensures
                    // re-processing a document never drops transitive attribution just because
                    // the OSV query was skipped on the second run (FIX C).
                    for tpkg in &transitive_cached_pkgs {
                        let normalized_version = normalize_version_for_osv(&tpkg.version);
                        let vuln_data_key =
                            VulnCacheKey::new(ecosystem, &tpkg.name, &normalized_version);
                        if let Some(vulns) = bg_ctx.transitive_vuln_data.get(&vuln_data_key) {
                            let vulns: Vec<_> = vulns.clone();
                            // No else: absence means no vulns, nothing to do.
                            let parents = inverse.get(&tpkg.name).cloned().unwrap_or_default();
                            if parents.is_empty() {
                                for v in &vulns {
                                    transitive_vulns_by_direct
                                        .entry_ref("(unknown)")
                                        .or_default()
                                        .push(TransitiveVuln {
                                            package_name: tpkg.name.clone(),
                                            package_version: tpkg.version.clone(),
                                            vulnerability: v.clone(),
                                        });
                                }
                            } else {
                                for parent in &parents {
                                    // Translate normalized parent name back to raw manifest name.
                                    let raw_parent = normalized_to_raw
                                        .get(parent.as_str())
                                        .cloned()
                                        .unwrap_or_else(|| parent.clone());
                                    for v in &vulns {
                                        transitive_vulns_by_direct
                                            .entry_ref(raw_parent.as_str())
                                            .or_default()
                                            .push(TransitiveVuln {
                                                package_name: tpkg.name.clone(),
                                                package_version: tpkg.version.clone(),
                                                vulnerability: v.clone(),
                                            });
                                    }
                                    tracing::debug!(
                                        "Background: Re-attributed (cached) {} transitive vulns from {}@{} to direct dep {}",
                                        vulns.len(),
                                        tpkg.name,
                                        tpkg.version,
                                        raw_parent
                                    );
                                }
                            }
                        } else {
                            tracing::debug!(
                                "vuln_cache hit without transitive_vuln_data for {}@{} — skipping attribution",
                                tpkg.name,
                                tpkg.version
                            );
                        }
                    }

                    // Dedup within each parent bucket.
                    for bucket in transitive_vulns_by_direct.values_mut() {
                        bucket.sort_by(|a, b| {
                            (&a.package_name, &a.package_version, &a.vulnerability.id).cmp(&(
                                &b.package_name,
                                &b.package_version,
                                &b.vulnerability.id,
                            ))
                        });
                        bucket.dedup_by(|a, b| {
                            a.package_name == b.package_name
                                && a.package_version == b.package_version
                                && a.vulnerability.id == b.vulnerability.id
                        });
                    }

                    // Write per-document transitive findings into the DocumentState so they are
                    // isolated from other workspaces that share the same global version_cache.
                    if let Some(mut doc) = bg_ctx.documents.get_mut(&bg_ctx.uri) {
                        doc.transitive_vulns_by_direct = transitive_vulns_by_direct;
                    }
                }

                tracing::info!(
                    "Background: Cached vulnerability info for {} packages",
                    updated_count
                );

                // Refresh UI with new vulnerability data
                tracing::debug!("Background: Refreshing inlay hints after vulnerability check");
                client
                    .send_request::<request::InlayHintRefreshRequest>(())
                    .await
                    .ok();
                client
                    .send_request::<request::WorkspaceDiagnosticRefresh>(())
                    .await
                    .ok();

                tracing::info!("Background: Vulnerability check complete, UI updated");
            }
            Err(e) => {
                tracing::warn!(
                    "Background: Failed to fetch vulnerabilities from OSV.dev: {}",
                    e
                );
            }
        }
    }

    async fn process_document(&self, uri: &Url, content: &str) {
        self.create_processing_context()
            .process_document(uri, content)
            .await;
    }

    /// Generate a vulnerability report for a document
    async fn generate_vulnerability_report(
        &self,
        arguments: &[serde_json::Value],
    ) -> serde_json::Value {
        // Parse arguments
        let format = arguments
            .first()
            .and_then(|v| v.get("format"))
            .and_then(|v| v.as_str())
            .unwrap_or("json");

        let uri_str = arguments
            .first()
            .and_then(|v| v.get("uri"))
            .and_then(|v| v.as_str());

        // Find the document
        let (uri, doc_state) = if let Some(uri_str) = uri_str {
            if let Ok(uri) = Url::parse(uri_str) {
                if let Some(doc) = self.documents.get(&uri) {
                    (uri.clone(), Some((doc.file_type, doc.dependencies.clone())))
                } else {
                    (uri, None)
                }
            } else {
                return serde_json::json!({
                    "error": "Invalid URI format"
                });
            }
        } else {
            // Use the first open document
            if let Some(entry) = self.documents.iter().next() {
                (
                    entry.key().clone(),
                    Some((entry.value().file_type, entry.value().dependencies.clone())),
                )
            } else {
                return serde_json::json!({
                    "error": "No open documents"
                });
            }
        };

        let Some((file_type, dependencies)) = doc_state else {
            return serde_json::json!({
                "error": "Document not found or not yet processed"
            });
        };

        // Collect vulnerabilities from the version cache
        let mut vulnerabilities: Vec<VulnerabilityReportEntry> = Vec::new();
        let mut summary = VulnerabilitySummary::default();

        for dep in &dependencies {
            let cache_key = dep_cache_key(dep, file_type);
            if let Some(info) = self.version_cache.get(&cache_key) {
                for vuln in &info.vulnerabilities {
                    summary.total += 1;
                    match vuln.severity {
                        VulnerabilitySeverity::Critical => summary.critical += 1,
                        VulnerabilitySeverity::High => summary.high += 1,
                        VulnerabilitySeverity::Medium => summary.medium += 1,
                        VulnerabilitySeverity::Low => summary.low += 1,
                    }

                    vulnerabilities.push(VulnerabilityReportEntry {
                        package: dep.name.clone(),
                        version: dep.version.clone(),
                        id: vuln.id.clone(),
                        severity: format!("{:?}", vuln.severity).to_lowercase(),
                        description: vuln.description.clone(),
                        url: vuln.url.clone(),
                    });
                }
            }
        }

        // Generate report based on format
        match format {
            "markdown" => {
                let md = fmt_markdown_report(&uri, &summary, &vulnerabilities).to_string();
                serde_json::json!({
                    "format": "markdown",
                    "content": md
                })
            }
            _ => {
                // Default to JSON
                serde_json::json!({
                    "summary": summary,
                    "vulnerabilities": vulnerabilities,
                    "file": String::from(uri) // .to_string() clones the inner String of the Url
                })
            }
        }
    }
}

/// Format the hover content for a dependency with version info.
///
/// Extracted from the hover handler to enable unit testing.
fn format_hover_content(
    dep: &crate::parsers::Dependency,
    file_type: FileType,
    info: &VersionInfo,
    doc_transitive_vulns: &[crate::registries::TransitiveVuln],
) -> String {
    let dep_name = &*dep.name;
    let dep_version = dep.effective_version();

    fmt::from_fn(|f| {
        writeln!(f, "## {dep_name}\n")?;

        if let Some(desc) = &info.description {
            writeln!(f, "{desc}\n")?;
        }

        // Current version with release date
        let current_date_str = info
            .get_release_date(dep_version)
            .map(|dt| format!(" ({})", fmt_release_age(dt)))
            .unwrap_or_default();
        writeln!(f, "**Current:** {dep_version}{current_date_str}")?;

        if let Some(latest) = info.latest.as_deref() {
            let latest_date_str = info
                .get_release_date(latest)
                .map(|dt| format!(" ({})", fmt_release_age(dt)))
                .unwrap_or_default();
            writeln!(f, "**Latest:** {latest}{latest_date_str}")?;
        }

        if let Some(license) = info.license.as_deref() {
            writeln!(f, "**License:** {license}")?;
        }

        if let Some(repo) = info.repository.as_deref() {
            writeln!(f, "\n[Repository]({repo})")?;
        }

        if let Some(homepage) = info.homepage.as_deref() {
            writeln!(f, "[Homepage]({homepage})")?;
        }

        if dep.registry.is_none() {
            let registry_url = file_type.fmt_registry_package_url(dep_name);
            writeln!(f, "[View on {}]({registry_url})", file_type.registry_name())?;
        }

        // Add vulnerability information if present
        if !info.vulnerabilities.is_empty() {
            let vulns_count = info.vulnerabilities.len();
            let suffix = if vulns_count == 1 {
                "Vulnerability"
            } else {
                "Vulnerabilities"
            };
            writeln!(f, "\n### ⚠ {vulns_count} Security {suffix}")?;

            for vuln in &info.vulnerabilities {
                let (severity_icon, severity_str) = match vuln.severity {
                    VulnerabilitySeverity::Critical => ("⚠", "CRITICAL"),
                    VulnerabilitySeverity::High => ("▲", "HIGH"),
                    VulnerabilitySeverity::Medium => ("●", "MEDIUM"),
                    VulnerabilitySeverity::Low => ("○", "LOW"),
                };

                let id = &*vuln.id;
                if let Some(url) = vuln.url.as_deref() {
                    writeln!(f, "\n#### [{id}]({url}) - {severity_icon} {severity_str}")?;
                } else {
                    writeln!(f, "\n#### {id} - {severity_icon} {severity_str}")?;
                }
                f.write_str(&vuln.description)?;
            }
        }

        // Add transitive vulnerability information if present.
        // Uses per-document attribution (not global version_cache) to avoid cross-workspace leaks.
        if !doc_transitive_vulns.is_empty() {
            writeln!(f, "\n## Transitive vulnerabilities")?;
            for t in doc_transitive_vulns {
                let link = match &t.vulnerability.url {
                    Some(u) => format!("[{}]({u})", t.vulnerability.id),
                    None => t.vulnerability.id.clone(),
                };
                let sev = t.vulnerability.severity.as_str();
                let name = &t.package_name;
                let ver = &t.package_version;
                let desc = &t.vulnerability.description;
                writeln!(f, "- {link} **{sev}** — {name}@{ver}: {desc}")?;
            }
        }

        Ok(())
    })
    .to_string()
}

#[tower_lsp::async_trait]
impl LanguageServer for DependiBackend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Parse configuration from initialization options
        let config = Config::from_init_options(params.initialization_options);
        tracing::info!("Configuration: {config:?}");

        // Register token providers from npm scoped registry config
        for (scope, scoped_config) in &config.registries.npm.scoped {
            if let Some(auth) = &scoped_config.auth
                && auth.is_configured()
            {
                if let Some(token) = auth.get_token() {
                    let provider = Arc::new(EnvTokenProvider::new(token.clone()));
                    self.token_manager
                        .register(scoped_config.url.clone(), provider)
                        .await;
                    tracing::info!(
                        "Registered auth provider for npm scope @{scope} -> {} (token: {})",
                        scoped_config.url,
                        fmt_redact_token(&token)
                    );
                } else {
                    tracing::warn!(
                        "Auth configured for npm scope @{scope} but token not found in env var {}",
                        auth.variable
                    );
                }
            }
        }

        tracing::info!(
            "Token manager initialized with {} providers",
            self.token_manager.provider_count().await
        );

        // Reconfigure npm registry with custom settings if provided
        {
            let new_npm_registry = NpmRegistry::with_client_and_config(
                Arc::clone(&self.http_client),
                &config.registries.npm,
            );
            let mut registry = self.npm_registry.write().await;
            *registry = new_npm_registry;
            tracing::info!(
                "npm registry configured with base URL: {}",
                config.registries.npm.url
            );
        }

        // Reconfigure Maven Central registry with custom base URL if provided
        {
            let new_maven = MavenCentralRegistry::with_client_and_config(
                Arc::clone(&self.http_client),
                &config.registries.maven,
            );
            let mut registry = self.maven_central.write().await;
            *registry = new_maven;
            tracing::info!(
                "Maven Central registry configured with base URL: {}",
                config.registries.maven.url
            );
        }

        // Configure Cargo alternative registries
        if !config.registries.cargo.registries.is_empty() {
            // Read tokens from ~/.cargo/credentials.toml (fallback auth source)
            let credential_tokens = {
                let cargo_home = std::env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")));

                let mut tokens = HashMap::new();
                if let Some(cargo_home) = cargo_home {
                    let cred_path = cargo_home.join("credentials.toml");
                    let content = if let Ok(c) = tokio::fs::read_to_string(&cred_path).await {
                        c
                    } else {
                        let alt_path = cargo_home.join("credentials");
                        tokio::fs::read_to_string(&alt_path)
                            .await
                            .unwrap_or_default()
                    };

                    if !content.is_empty() {
                        tokens = cargo_credentials::parse_credentials_content(&content);
                        tracing::debug!("Loaded {} tokens from Cargo credentials", tokens.len());
                    }
                }
                tokens
            };

            for (registry_name, registry_config) in &config.registries.cargo.registries {
                // Token priority: LSP config auth > credentials.toml
                let token = registry_config
                    .auth
                    .as_ref()
                    .and_then(|auth| auth.get_token())
                    .or_else(|| credential_tokens.get(registry_name).cloned());

                if let Some(t) = token.as_deref() {
                    tracing::info!(
                        "Using auth token for Cargo registry '{registry_name}' (token: {})",
                        fmt_redact_token(t)
                    );
                }

                let registry = Arc::new(CargoSparseRegistry::with_client_and_config(
                    Arc::clone(&self.http_client),
                    registry_config.index_url.clone(),
                    token,
                ));

                self.cargo_custom_registries
                    .insert(registry_name.clone(), registry);
                tracing::info!(
                    "Configured Cargo alternative registry: {registry_name} -> {}",
                    registry_config.index_url
                );
            }

            tracing::info!(
                "Cargo alternative registries configured: {}",
                self.cargo_custom_registries.len()
            );
        }

        // Store the configuration
        if let Ok(mut cfg) = self.config.write() {
            *cfg = config;
        }

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "dependi-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                inlay_hint_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["\"".to_string(), "=".to_string()]),
                    ..Default::default()
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["dependi/generateReport".to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Dependi LSP initialized")
            .await;

        // Verify all registries share the same HTTP client
        let base_client = self.crates_io.http_client();
        debug_assert!(Arc::ptr_eq(
            &base_client,
            &self.npm_registry.read().await.http_client()
        ));
        debug_assert!(Arc::ptr_eq(&base_client, &self.pypi.http_client()));
        debug_assert!(Arc::ptr_eq(&base_client, &self.go_proxy.http_client()));
        debug_assert!(Arc::ptr_eq(&base_client, &self.packagist.http_client()));
        debug_assert!(Arc::ptr_eq(&base_client, &self.pub_dev.http_client()));
        debug_assert!(Arc::ptr_eq(&base_client, &self.nuget.http_client()));
        debug_assert!(Arc::ptr_eq(&base_client, &self.rubygems.http_client()));

        tracing::info!("Dependi LSP initialized");
    }

    async fn shutdown(&self) -> Result<()> {
        tracing::info!("Dependi LSP shutting down");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let content = params.text_document.text;

        tracing::debug!("Document opened: {}", uri);
        self.process_document(&uri, &content).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        use std::sync::atomic::Ordering;

        let uri = params.text_document.uri;

        // With FULL sync, we get the entire document content
        if let Some(change) = params.content_changes.into_iter().next() {
            tracing::debug!("Document changed: {}", uri);

            // Store pending content
            self.pending_changes
                .insert(uri.clone(), change.text.clone());

            // Cancel any existing debounce task for this URI
            if let Some((_, (_, handle))) = self.debounce_tasks.remove(&uri) {
                handle.abort();
            }

            // Increment generation for this new task
            let generation = self.debounce_generation.fetch_add(1, Ordering::SeqCst) + 1;

            // Get debounce delay from config (default 200ms)
            let debounce_ms = self
                .config
                .read()
                .map(|c| c.cache.debounce_ms)
                .unwrap_or(200);

            // Create processing context for the spawned task
            let ctx = self.create_processing_context();
            let uri_clone = uri.clone();
            let content = change.text;
            let pending_changes = Arc::clone(&self.pending_changes);
            let debounce_tasks = Arc::clone(&self.debounce_tasks);

            // Spawn debounced processing task
            let handle = tokio::spawn(async move {
                // Wait for debounce period
                tokio::time::sleep(Duration::from_millis(debounce_ms)).await;

                // Check if this is still the latest content (no newer changes)
                let should_process = pending_changes
                    .get(&uri_clone)
                    .map(|v| *v == content)
                    .unwrap_or(false);

                if should_process {
                    tracing::debug!("Processing document after debounce: {}", uri_clone);
                    ctx.process_document(&uri_clone, &content).await;
                    pending_changes.remove(&uri_clone);
                }

                // Clean up task handle only if generation matches (no newer task spawned)
                debounce_tasks
                    .remove_if(&uri_clone, |_, (stored_gen, _)| *stored_gen == generation);
            });

            // Store new task handle with generation
            self.debounce_tasks.insert(uri, (generation, handle));
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;

        // Re-process on save if we have the text
        if let Some(text) = params.text {
            tracing::debug!("Document saved: {}", uri);

            // Cancel any pending debounce task for this URI (save bypasses debounce)
            if let Some((_, (_, handle))) = self.debounce_tasks.remove(&uri) {
                handle.abort();
            }
            self.pending_changes.remove(&uri);

            // Process immediately on save
            self.process_document(&uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        tracing::debug!("Document closed: {}", uri);

        // Cancel any pending debounce task for this URI
        if let Some((_, (_, handle))) = self.debounce_tasks.remove(&uri) {
            handle.abort();
        }
        self.pending_changes.remove(&uri);

        self.documents.remove(&uri);

        // Clear diagnostics for this document
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let Ok(config) = self.config.read() else {
            return Ok(Some(vec![]));
        };
        if !config.inlay_hints.enabled {
            return Ok(Some(vec![]));
        }
        let show_up_to_date = config.inlay_hints.show_up_to_date;
        let ignored_packages = config.ignore.clone();
        drop(config);

        let uri = &params.text_document.uri;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(Some(vec![]));
        };

        let file_type = doc.file_type;
        let hints: Vec<InlayHint> = doc
            .dependencies
            .iter()
            .filter(|dep| {
                // Only show hints for dependencies in the visible range
                (params.range.start.line..=params.range.end.line).contains(&dep.version_span.line)
            })
            .filter(|dep| !crate::config::is_package_ignored(&dep.name, &ignored_packages))
            .filter_map(|dep| {
                let cache_key = dep_cache_key(dep, file_type);
                let version_info = self.version_cache.get(&cache_key);
                let hint = create_inlay_hint(dep, version_info.as_ref(), file_type);

                // Optionally filter out up-to-date hints
                if !show_up_to_date {
                    let label_text = match &hint.label {
                        InlayHintLabel::String(s) => s.clone(),
                        InlayHintLabel::LabelParts(parts) => {
                            parts.iter().map(|p| p.value.as_str()).collect()
                        }
                    };
                    if label_text.contains("[OK]") {
                        return None;
                    }
                }
                Some(hint)
            })
            .collect();

        tracing::debug!("Returning {} inlay hints for {}", hints.len(), uri);
        Ok(Some(hints))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let uri = &params.text_document.uri;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(Some(vec![]));
        };

        let links = create_document_links(&doc.dependencies, &doc.file_type);
        tracing::debug!("Returning {} document links for {}", links.len(), uri);
        Ok(Some(links))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let file_type = doc.file_type;

        enum HoveredSpan {
            Name,
            Version,
        }

        // Find dependency at this position
        let Some((dep, hovered_span)) = doc.dependencies.iter().find_map(|d| {
            if d.name_span.contains_lsp_position(&position) {
                Some((d.clone(), HoveredSpan::Name))
            } else if d.version_span.contains_lsp_position(&position) {
                Some((d.clone(), HoveredSpan::Version))
            } else {
                None
            }
        }) else {
            return Ok(None);
        };
        let dep_name = &*dep.name;
        let hovered_span = match hovered_span {
            HoveredSpan::Name => dep.name_span,
            HoveredSpan::Version => dep.version_span,
        };

        // Snapshot per-document transitive vulns for this dependency before dropping the lock.
        let doc_transitive_vulns: Vec<crate::registries::TransitiveVuln> = doc
            .transitive_vulns_by_direct
            .get(dep_name)
            .cloned()
            .unwrap_or_default();

        // Drop the lock before async call
        drop(doc);

        // Get version info
        let version_info = self
            .get_version_info(file_type, dep_name, dep.registry.as_deref())
            .await;

        let content = match version_info {
            Some(info) => format_hover_content(&dep, file_type, &info, &doc_transitive_vulns),
            None => format!("## {dep_name}\n\nCould not fetch package information."),
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(Range {
                start: Position {
                    line: hovered_span.line,
                    character: hovered_span.line_start,
                },
                end: Position {
                    line: hovered_span.line,
                    character: hovered_span.line_end,
                },
            }),
        }))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(Some(vec![]));
        };

        let file_type = doc.file_type;
        let cache_key_map: HashMap<String, String> = doc
            .dependencies
            .iter()
            .map(|dep| (dep.name.clone(), dep_cache_key(dep, file_type)))
            .collect();
        let actions = create_code_actions(
            &doc.dependencies,
            &self.version_cache,
            uri,
            params.range,
            file_type,
            |name| {
                cache_key_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| file_type.cache_key(name))
            },
        );

        Ok(Some(actions))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(Some(CompletionResponse::Array(vec![])));
        };

        let file_type = doc.file_type;
        let cache_key_map: HashMap<String, String> = doc
            .dependencies
            .iter()
            .map(|dep| (dep.name.clone(), dep_cache_key(dep, file_type)))
            .collect();
        let completions =
            get_completions(&doc.dependencies, position, &self.version_cache, |name| {
                cache_key_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| file_type.cache_key(name))
            });

        match completions {
            Some(items) => Ok(Some(CompletionResponse::Array(items))),
            None => Ok(Some(CompletionResponse::Array(vec![]))),
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        match params.command.as_str() {
            "dependi/generateReport" => {
                let report = self.generate_vulnerability_report(&params.arguments).await;
                Ok(Some(report))
            }
            _ => {
                tracing::warn!("Unknown command: {}", params.command);
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::{Dependency, Span};

    fn make_dep(name: &str, version: &str, resolved: Option<&str>) -> Dependency {
        Dependency {
            name: name.to_string(),
            version: version.to_string(),
            name_span: Span {
                line: 0,
                line_start: 0,
                line_end: name.len() as u32,
            },
            version_span: Span {
                line: 0,
                line_start: name.len() as u32 + 3,
                line_end: name.len() as u32 + 3 + version.len() as u32,
            },
            dev: false,
            optional: false,
            registry: None,
            resolved_version: resolved.map(str::to_string),
        }
    }

    #[test]
    fn test_hover_uses_effective_version_not_manifest_specifier() {
        let dep = make_dep("serde", "^1.0", Some("1.0.200"));
        let info = VersionInfo {
            latest: Some("1.0.210".to_string()),
            ..Default::default()
        };

        let content = format_hover_content(&dep, FileType::Cargo, &info, &[]);

        // Must show the resolved version, not the manifest specifier
        assert!(
            content.contains("**Current:** 1.0.200"),
            "Hover should show effective_version '1.0.200', got:\n{content}"
        );
        assert!(
            !content.contains("**Current:** ^1.0"),
            "Hover should NOT show manifest specifier '^1.0'"
        );
    }

    #[test]
    fn test_hover_uses_manifest_version_when_no_lockfile() {
        let dep = make_dep("tokio", "1.35.0", None);
        let info = VersionInfo {
            latest: Some("1.40.0".to_string()),
            ..Default::default()
        };

        let content = format_hover_content(&dep, FileType::Cargo, &info, &[]);

        assert!(
            content.contains("**Current:** 1.35.0"),
            "Without lockfile, should show manifest version, got:\n{content}"
        );
    }

    #[test]
    fn test_hover_release_date_uses_effective_version() {
        use chrono::{TimeZone, Utc};

        let dep = make_dep("serde", "^1.0", Some("1.0.200"));
        let release_date = Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();

        let mut info = VersionInfo {
            latest: Some("1.0.200".to_string()),
            ..Default::default()
        };
        // Insert release date keyed by the resolved version, NOT the specifier
        info.release_dates
            .insert("1.0.200".to_string(), release_date);

        let content = format_hover_content(&dep, FileType::Cargo, &info, &[]);

        // The release date should be found (keyed by effective_version "1.0.200")
        assert!(
            content.contains("2025-01-15") || content.contains("ago"),
            "Release date should be found via effective_version lookup, got:\n{content}"
        );
    }

    #[test]
    fn test_hover_lists_transitive_vulnerabilities() {
        use crate::registries::{TransitiveVuln, Vulnerability, VulnerabilitySeverity};

        let dep = make_dep("react", "^18.0", Some("18.2.0"));
        let info = VersionInfo {
            latest: Some("18.2.0".to_string()),
            ..Default::default()
        };
        let transitive_vulns = vec![TransitiveVuln {
            package_name: "scheduler".to_string(),
            package_version: "1.2.3".to_string(),
            vulnerability: Vulnerability {
                id: "CVE-1".to_string(),
                severity: VulnerabilitySeverity::High,
                description: "desc".to_string(),
                url: None,
            },
        }];
        let content = format_hover_content(&dep, FileType::Npm, &info, &transitive_vulns);
        assert!(
            content.contains("Transitive vulnerabilities"),
            "Hover should contain transitive section, got:\n{content}"
        );
        assert!(
            content.contains("scheduler@1.2.3"),
            "Hover should contain transitive package, got:\n{content}"
        );
        assert!(
            content.contains("CVE-1"),
            "Hover should contain transitive vuln id, got:\n{content}"
        );
    }
}
