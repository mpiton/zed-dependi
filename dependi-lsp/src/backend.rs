use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::cache::HybridCache;
use crate::config::Config;
use crate::parsers::cargo::CargoParser;
use crate::parsers::csharp::CsharpParser;
use crate::parsers::dart::DartParser;
use crate::parsers::go::GoParser;
use crate::parsers::npm::NpmParser;
use crate::parsers::php::PhpParser;
use crate::parsers::python::PythonParser;
use crate::parsers::ruby::RubyParser;
use crate::parsers::{Dependency, Parser};
use crate::providers::code_actions::create_code_actions;
use crate::providers::completion::{format_release_age, get_completions};
use crate::providers::diagnostics::create_diagnostics;
use crate::providers::inlay_hints::create_inlay_hint;
use crate::registries::crates_io::CratesIoRegistry;
use crate::registries::go_proxy::GoProxyRegistry;
use crate::registries::http_client::create_shared_client;
use crate::registries::npm::NpmRegistry;
use crate::registries::nuget::NuGetRegistry;
use crate::registries::packagist::PackagistRegistry;
use crate::registries::pub_dev::PubDevRegistry;
use crate::registries::pypi::PyPiRegistry;
use crate::registries::rubygems::RubyGemsRegistry;
use crate::registries::{Registry, VersionInfo, VulnerabilitySeverity};
use crate::vulnerabilities::cache::VulnerabilityCache;
use crate::vulnerabilities::osv::OsvClient;
use crate::vulnerabilities::{Ecosystem, VulnerabilityQuery};

/// File type for determining which parser/registry to use
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    Cargo,
    Npm,
    Python,
    Go,
    Php,
    Dart,
    Csharp,
    Ruby,
}

impl FileType {
    /// Convert to vulnerability ecosystem
    pub fn to_ecosystem(self) -> Ecosystem {
        match self {
            FileType::Cargo => Ecosystem::CratesIo,
            FileType::Npm => Ecosystem::Npm,
            FileType::Python => Ecosystem::PyPI,
            FileType::Go => Ecosystem::Go,
            FileType::Php => Ecosystem::Packagist,
            FileType::Dart => Ecosystem::Pub,
            FileType::Csharp => Ecosystem::NuGet,
            FileType::Ruby => Ecosystem::RubyGems,
        }
    }
}

/// Document state with parsed dependencies
struct DocumentState {
    /// Parsed dependencies
    dependencies: Vec<Dependency>,
    /// Type of dependency file
    file_type: FileType,
}

pub struct DependiBackend {
    client: Client,
    /// Configuration
    config: RwLock<Config>,
    /// Cache for documents and their parsed state
    documents: DashMap<Url, DocumentState>,
    /// Cache for version information (keyed by "registry:package")
    version_cache: Arc<HybridCache>,
    /// Parsers
    cargo_parser: CargoParser,
    npm_parser: NpmParser,
    python_parser: PythonParser,
    go_parser: GoParser,
    php_parser: PhpParser,
    dart_parser: DartParser,
    csharp_parser: CsharpParser,
    ruby_parser: RubyParser,
    /// Registry clients
    crates_io: Arc<CratesIoRegistry>,
    npm_registry: Arc<NpmRegistry>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    /// Vulnerability scanning
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
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

        Self {
            client,
            config: RwLock::new(Config::default()),
            documents: DashMap::new(),
            version_cache: Arc::new(HybridCache::new()),
            cargo_parser: CargoParser::new(),
            npm_parser: NpmParser::new(),
            python_parser: PythonParser::new(),
            go_parser: GoParser::new(),
            php_parser: PhpParser::new(),
            dart_parser: DartParser::new(),
            csharp_parser: CsharpParser::new(),
            ruby_parser: RubyParser::new(),
            crates_io: Arc::new(CratesIoRegistry::with_client(Arc::clone(&http_client))),
            npm_registry: Arc::new(NpmRegistry::with_client(Arc::clone(&http_client))),
            pypi: Arc::new(PyPiRegistry::with_client(Arc::clone(&http_client))),
            go_proxy: Arc::new(GoProxyRegistry::with_client(Arc::clone(&http_client))),
            packagist: Arc::new(PackagistRegistry::with_client(Arc::clone(&http_client))),
            pub_dev: Arc::new(PubDevRegistry::with_client(Arc::clone(&http_client))),
            nuget: Arc::new(NuGetRegistry::with_client(Arc::clone(&http_client))),
            rubygems: Arc::new(RubyGemsRegistry::with_client(http_client)),
            osv_client: Arc::new(OsvClient::default()),
            vuln_cache: Arc::new(VulnerabilityCache::new()),
        }
    }

    /// Detect file type from URI
    fn detect_file_type(uri: &Url) -> Option<FileType> {
        let path = uri.path();
        if path.ends_with("Cargo.toml") {
            Some(FileType::Cargo)
        } else if path.ends_with("package.json") {
            Some(FileType::Npm)
        } else if path.ends_with("requirements.txt")
            || path.ends_with("requirements-dev.txt")
            || path.ends_with("requirements-test.txt")
            || path.ends_with("pyproject.toml")
        {
            Some(FileType::Python)
        } else if path.ends_with("go.mod") {
            Some(FileType::Go)
        } else if path.ends_with("composer.json") {
            Some(FileType::Php)
        } else if path.ends_with("pubspec.yaml") {
            Some(FileType::Dart)
        } else if path.ends_with(".csproj") {
            Some(FileType::Csharp)
        } else if path.ends_with("Gemfile") {
            Some(FileType::Ruby)
        } else {
            None
        }
    }

    /// Parse a document and extract dependencies
    fn parse_document(&self, uri: &Url, content: &str) -> Vec<Dependency> {
        match Self::detect_file_type(uri) {
            Some(FileType::Cargo) => self.cargo_parser.parse(content),
            Some(FileType::Npm) => self.npm_parser.parse(content),
            Some(FileType::Python) => self.python_parser.parse(content),
            Some(FileType::Go) => self.go_parser.parse(content),
            Some(FileType::Php) => self.php_parser.parse(content),
            Some(FileType::Dart) => self.dart_parser.parse(content),
            Some(FileType::Csharp) => self.csharp_parser.parse(content),
            Some(FileType::Ruby) => self.ruby_parser.parse(content),
            None => vec![],
        }
    }

    /// Get cache key for a package (includes registry prefix)
    fn cache_key(file_type: FileType, package_name: &str) -> String {
        match file_type {
            FileType::Cargo => format!("crates:{}", package_name),
            FileType::Npm => format!("npm:{}", package_name),
            FileType::Python => format!("pypi:{}", package_name),
            FileType::Go => format!("go:{}", package_name),
            FileType::Php => format!("packagist:{}", package_name),
            FileType::Dart => format!("pub:{}", package_name),
            FileType::Csharp => format!("nuget:{}", package_name),
            FileType::Ruby => format!("rubygems:{}", package_name),
        }
    }

    /// Fetch version info for a package (with caching)
    async fn get_version_info(
        &self,
        file_type: FileType,
        package_name: &str,
    ) -> Option<VersionInfo> {
        let cache_key = Self::cache_key(file_type, package_name);

        // Check cache first
        if let Some(cached) = self.version_cache.get(&cache_key) {
            return Some(cached);
        }

        // Fetch from appropriate registry
        let result = match file_type {
            FileType::Cargo => self.crates_io.get_version_info(package_name).await,
            FileType::Npm => self.npm_registry.get_version_info(package_name).await,
            FileType::Python => self.pypi.get_version_info(package_name).await,
            FileType::Go => self.go_proxy.get_version_info(package_name).await,
            FileType::Php => self.packagist.get_version_info(package_name).await,
            FileType::Dart => self.pub_dev.get_version_info(package_name).await,
            FileType::Csharp => self.nuget.get_version_info(package_name).await,
            FileType::Ruby => self.rubygems.get_version_info(package_name).await,
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

    /// Fetch vulnerabilities from OSV.dev for all dependencies
    async fn fetch_vulnerabilities(&self, dependencies: &[Dependency], file_type: FileType) {
        use crate::vulnerabilities::cache::VulnCacheKey;

        let ecosystem = file_type.to_ecosystem();

        // Build queries for packages not in vulnerability cache
        let queries: Vec<VulnerabilityQuery> = dependencies
            .iter()
            .filter(|dep| {
                let vuln_key = VulnCacheKey::new(ecosystem, &dep.name, &dep.version);
                !self.vuln_cache.contains(&vuln_key)
            })
            .map(|dep| VulnerabilityQuery {
                ecosystem,
                package_name: dep.name.clone(),
                version: dep.version.clone(),
            })
            .collect();

        if queries.is_empty() {
            tracing::debug!("All vulnerability info cached, skipping OSV query");
            return;
        }

        tracing::info!("Querying OSV.dev for {} packages", queries.len());

        // Batch query OSV.dev
        match self.osv_client.query_batch(&queries).await {
            Ok(results) => {
                // Update vulnerability cache and version_cache with results
                for (query, result) in queries.iter().zip(results.iter()) {
                    // Mark this package as queried in vuln_cache
                    let vuln_key =
                        VulnCacheKey::new(ecosystem, &query.package_name, &query.version);
                    self.vuln_cache.insert(vuln_key);

                    // Store vulnerabilities and deprecated status in version_cache
                    let cache_key = Self::cache_key(file_type, &query.package_name);
                    if let Some(mut info) = self.version_cache.get(&cache_key) {
                        info.vulnerabilities = result.vulnerabilities.clone();
                        info.deprecated = result.deprecated;
                        if result.deprecated {
                            tracing::info!(
                                "Package {} {} is deprecated (unmaintained)",
                                query.package_name,
                                query.version
                            );
                        }
                        tracing::debug!(
                            "Updated {} {} with {} vulnerabilities, deprecated={}",
                            query.package_name,
                            query.version,
                            result.vulnerabilities.len(),
                            result.deprecated
                        );
                        self.version_cache.insert(cache_key, info);
                    } else {
                        tracing::warn!(
                            "Could not update vulnerabilities for {}: not found in version cache",
                            query.package_name
                        );
                    }
                }
                tracing::info!("Cached vulnerability info for {} packages", queries.len());
            }
            Err(e) => {
                tracing::warn!("Failed to fetch vulnerabilities from OSV.dev: {}", e);
            }
        }
    }

    /// Process a document: parse and fetch version info
    async fn process_document(&self, uri: &Url, content: &str) {
        let Some(file_type) = Self::detect_file_type(uri) else {
            return;
        };

        let dependencies = self.parse_document(uri, content);

        tracing::info!(
            "Parsed {} dependencies from {}",
            dependencies.len(),
            uri.path()
        );

        // Clone Arc references for async tasks
        let crates_io = Arc::clone(&self.crates_io);
        let npm_registry = Arc::clone(&self.npm_registry);
        let pypi = Arc::clone(&self.pypi);
        let go_proxy = Arc::clone(&self.go_proxy);
        let packagist = Arc::clone(&self.packagist);
        let pub_dev = Arc::clone(&self.pub_dev);
        let nuget = Arc::clone(&self.nuget);
        let rubygems = Arc::clone(&self.rubygems);
        let cache = Arc::clone(&self.version_cache);

        let fetch_tasks: Vec<_> = dependencies
            .iter()
            .map(|dep| {
                let name = dep.name.clone();
                let cache_key = Self::cache_key(file_type, &name);
                let crates_io = Arc::clone(&crates_io);
                let npm_registry = Arc::clone(&npm_registry);
                let pypi = Arc::clone(&pypi);
                let go_proxy = Arc::clone(&go_proxy);
                let packagist = Arc::clone(&packagist);
                let pub_dev = Arc::clone(&pub_dev);
                let nuget = Arc::clone(&nuget);
                let rubygems = Arc::clone(&rubygems);
                let cache = Arc::clone(&cache);
                async move {
                    // Check cache first
                    if cache.get(&cache_key).is_some() {
                        return;
                    }
                    // Fetch from appropriate registry
                    let result = match file_type {
                        FileType::Cargo => crates_io.get_version_info(&name).await,
                        FileType::Npm => npm_registry.get_version_info(&name).await,
                        FileType::Python => pypi.get_version_info(&name).await,
                        FileType::Go => go_proxy.get_version_info(&name).await,
                        FileType::Php => packagist.get_version_info(&name).await,
                        FileType::Dart => pub_dev.get_version_info(&name).await,
                        FileType::Csharp => nuget.get_version_info(&name).await,
                        FileType::Ruby => rubygems.get_version_info(&name).await,
                    };
                    if let Ok(info) = result {
                        cache.insert(cache_key, info);
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

        // Fetch vulnerabilities from OSV.dev (if security is enabled)
        let security_enabled = self
            .config
            .read()
            .map(|c| c.security.enabled)
            .unwrap_or(true);

        if security_enabled && !dependencies.is_empty() {
            self.fetch_vulnerabilities(&dependencies, file_type).await;
        }

        // Store document state
        self.documents.insert(
            uri.clone(),
            DocumentState {
                dependencies: dependencies.clone(),
                file_type,
            },
        );

        // Publish diagnostics (if enabled)
        let (diagnostics_enabled, security_show_diags, min_severity) = self
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
                )
            })
            .unwrap_or((true, true, None));

        if diagnostics_enabled {
            // Pass min_severity filter only if security diagnostics are enabled
            let severity_filter = if security_show_diags {
                min_severity
            } else {
                None
            };
            let diagnostics = create_diagnostics(
                &dependencies,
                &self.version_cache,
                |name| Self::cache_key(file_type, name),
                severity_filter,
            );

            self.client
                .publish_diagnostics(uri.clone(), diagnostics, None)
                .await;
        }

        // Refresh inlay hints
        self.client
            .send_request::<request::InlayHintRefreshRequest>(())
            .await
            .ok();
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
            let cache_key = Self::cache_key(file_type, &dep.name);
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
                let md = generate_markdown_report(&uri, &summary, &vulnerabilities);
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
                    "file": uri.to_string()
                })
            }
        }
    }
}

/// Summary of vulnerabilities by severity
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct VulnerabilitySummary {
    total: u32,
    critical: u32,
    high: u32,
    medium: u32,
    low: u32,
}

/// Entry in the vulnerability report
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VulnerabilityReportEntry {
    package: String,
    version: String,
    id: String,
    severity: String,
    description: String,
    url: Option<String>,
}

/// Generate a markdown vulnerability report
fn generate_markdown_report(
    uri: &Url,
    summary: &VulnerabilitySummary,
    vulnerabilities: &[VulnerabilityReportEntry],
) -> String {
    let mut lines = vec![
        "# Vulnerability Report".to_string(),
        String::new(),
        format!("**File**: {}", uri.path()),
        format!("**Date**: {}", chrono::Local::now().format("%Y-%m-%d")),
        String::new(),
        "## Summary".to_string(),
        "| Severity | Count |".to_string(),
        "|----------|-------|".to_string(),
        format!("| ⚠ Critical | {} |", summary.critical),
        format!("| ▲ High | {} |", summary.high),
        format!("| ● Medium | {} |", summary.medium),
        format!("| ○ Low | {} |", summary.low),
        format!("| **Total** | **{}** |", summary.total),
        String::new(),
    ];

    if !vulnerabilities.is_empty() {
        lines.push("## Vulnerabilities".to_string());
        lines.push(String::new());

        // Group by package
        let mut current_package = String::new();
        for vuln in vulnerabilities {
            if vuln.package != current_package {
                current_package = vuln.package.clone();
                lines.push(format!("### {}@{}", vuln.package, vuln.version));
                lines.push(String::new());
            }

            let severity_icon = match vuln.severity.as_str() {
                "critical" => "⚠",
                "high" => "▲",
                "medium" => "●",
                _ => "○",
            };

            if let Some(url) = &vuln.url {
                lines.push(format!(
                    "- **[{}]({})** ({} {}): {}",
                    vuln.id,
                    url,
                    severity_icon,
                    vuln.severity.to_uppercase(),
                    vuln.description
                ));
            } else {
                lines.push(format!(
                    "- **{}** ({} {}): {}",
                    vuln.id,
                    severity_icon,
                    vuln.severity.to_uppercase(),
                    vuln.description
                ));
            }
        }
    } else {
        lines.push("## No vulnerabilities found".to_string());
        lines.push(String::new());
        lines.push("✅ All dependencies are free of known security vulnerabilities.".to_string());
    }

    lines.join("\n")
}

#[tower_lsp::async_trait]
impl LanguageServer for DependiBackend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Parse configuration from initialization options
        let config = Config::from_init_options(params.initialization_options);
        tracing::info!("Configuration: {:?}", config);

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
        debug_assert!(Arc::ptr_eq(&base_client, &self.npm_registry.http_client()));
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
        let uri = params.text_document.uri;

        // With FULL sync, we get the entire document content
        if let Some(change) = params.content_changes.into_iter().next() {
            tracing::debug!("Document changed: {}", uri);
            self.process_document(&uri, &change.text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;

        // Re-process on save if we have the text
        if let Some(text) = params.text {
            tracing::debug!("Document saved: {}", uri);
            self.process_document(&uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        tracing::debug!("Document closed: {}", uri);
        self.documents.remove(&uri);

        // Clear diagnostics for this document
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        // Check if inlay hints are enabled
        let config = self.config.read().unwrap();
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
                let line = dep.line;
                line >= params.range.start.line && line <= params.range.end.line
            })
            .filter(|dep| {
                // Skip ignored packages
                !ignored_packages.iter().any(|pattern| {
                    if pattern.contains('*') {
                        let parts: Vec<&str> = pattern.split('*').collect();
                        if parts.len() == 2 {
                            dep.name.starts_with(parts[0]) && dep.name.ends_with(parts[1])
                        } else {
                            dep.name.starts_with(parts[0])
                        }
                    } else {
                        dep.name == *pattern
                    }
                })
            })
            .filter_map(|dep| {
                let cache_key = Self::cache_key(file_type, &dep.name);
                let version_info = self.version_cache.get(&cache_key);
                let hint = create_inlay_hint(dep, version_info.as_ref());

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

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(doc) = self.documents.get(uri) else {
            return Ok(None);
        };

        let file_type = doc.file_type;

        // Find dependency at this position
        let dep = doc.dependencies.iter().find(|d| {
            d.line == position.line
                && position.character >= d.name_start
                && position.character <= d.version_end
        });

        let Some(dep) = dep.cloned() else {
            return Ok(None);
        };

        // Drop the lock before async call
        drop(doc);

        // Get version info
        let version_info = self.get_version_info(file_type, &dep.name).await;

        let content = match version_info {
            Some(info) => {
                let mut parts = vec![format!("## {}\n", dep.name)];

                if let Some(desc) = &info.description {
                    parts.push(format!("{}\n", desc));
                }

                // Current version with release date
                let current_date_str = info
                    .get_release_date(&dep.version)
                    .map(|dt| format!(" ({})", format_release_age(dt)))
                    .unwrap_or_default();
                parts.push(format!("**Current:** {}{}", dep.version, current_date_str));

                if let Some(latest) = &info.latest {
                    let latest_date_str = info
                        .get_release_date(latest)
                        .map(|dt| format!(" ({})", format_release_age(dt)))
                        .unwrap_or_default();
                    parts.push(format!("**Latest:** {}{}", latest, latest_date_str));
                }

                if let Some(license) = &info.license {
                    parts.push(format!("**License:** {}", license));
                }

                if let Some(repo) = &info.repository {
                    parts.push(format!("\n[Repository]({})", repo));
                }

                if let Some(homepage) = &info.homepage {
                    parts.push(format!("[Homepage]({})", homepage));
                }

                // Add vulnerability information if present
                if !info.vulnerabilities.is_empty() {
                    parts.push(format!(
                        "\n### ⚠ {} Security {}",
                        info.vulnerabilities.len(),
                        if info.vulnerabilities.len() == 1 {
                            "Vulnerability"
                        } else {
                            "Vulnerabilities"
                        }
                    ));

                    for vuln in &info.vulnerabilities {
                        let severity_icon = match vuln.severity {
                            VulnerabilitySeverity::Critical => "⚠",
                            VulnerabilitySeverity::High => "▲",
                            VulnerabilitySeverity::Medium => "●",
                            VulnerabilitySeverity::Low => "○",
                        };
                        let severity_str = match vuln.severity {
                            VulnerabilitySeverity::Critical => "CRITICAL",
                            VulnerabilitySeverity::High => "HIGH",
                            VulnerabilitySeverity::Medium => "MEDIUM",
                            VulnerabilitySeverity::Low => "LOW",
                        };

                        if let Some(url) = &vuln.url {
                            parts.push(format!(
                                "\n#### [{}]({}) - {} {}",
                                vuln.id, url, severity_icon, severity_str
                            ));
                        } else {
                            parts.push(format!(
                                "\n#### {} - {} {}",
                                vuln.id, severity_icon, severity_str
                            ));
                        }
                        parts.push(vuln.description.clone());
                    }
                }

                parts.join("\n")
            }
            None => format!("## {}\n\nCould not fetch package information.", dep.name),
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: Some(Range {
                start: Position {
                    line: dep.line,
                    character: dep.name_start,
                },
                end: Position {
                    line: dep.line,
                    character: dep.version_end,
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
        let actions = create_code_actions(
            &doc.dependencies,
            &self.version_cache,
            uri,
            params.range,
            file_type,
            |name| Self::cache_key(file_type, name),
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
        let completions =
            get_completions(&doc.dependencies, position, &self.version_cache, |name| {
                Self::cache_key(file_type, name)
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
