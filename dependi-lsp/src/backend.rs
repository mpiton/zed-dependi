use std::sync::{Arc, RwLock};
use std::time::Duration;

use dashmap::DashMap;
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
use crate::parsers::npm::NpmParser;
use crate::parsers::php::PhpParser;
use crate::parsers::python::PythonParser;
use crate::parsers::ruby::RubyParser;
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
use crate::reports::{VulnerabilityReportEntry, VulnerabilitySummary, generate_markdown_report};
use crate::vulnerabilities::VulnerabilityQuery;
use crate::vulnerabilities::cache::VulnerabilityCache;
use crate::vulnerabilities::osv::OsvClient;

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
    crates_io: Arc<CratesIoRegistry>,
    npm_registry: Arc<tokio::sync::RwLock<NpmRegistry>>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
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
            None => vec![],
        }
    }

    async fn process_document(&self, uri: &Url, content: &str) {
        let Some(file_type) = FileType::detect(uri) else {
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
                let cache_key = file_type.cache_key(&name);
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
                        FileType::Npm => npm_registry.read().await.get_version_info(&name).await,
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

        // Store document state IMMEDIATELY (before vulnerability check)
        self.documents.insert(
            uri.clone(),
            DocumentState {
                dependencies: dependencies.clone(),
                file_type,
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
            let diagnostics = create_diagnostics(
                &dependencies,
                &self.version_cache,
                |name| file_type.cache_key(name),
                severity_filter,
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
            let client_clone = self.client.clone();

            tokio::spawn(async move {
                DependiBackend::fetch_vulnerabilities_background(
                    dependencies_clone,
                    file_type,
                    cache_clone,
                    osv_client_clone,
                    vuln_cache_clone,
                    client_clone,
                )
                .await;
            });
        }
    }
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
    /// Registry clients
    crates_io: Arc<CratesIoRegistry>,
    /// npm registry (tokio::sync::RwLock-wrapped to allow reconfiguration during initialize)
    npm_registry: Arc<tokio::sync::RwLock<NpmRegistry>>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
    pub_dev: Arc<PubDevRegistry>,
    nuget: Arc<NuGetRegistry>,
    rubygems: Arc<RubyGemsRegistry>,
    /// Shared HTTP client for creating new registry instances
    http_client: Arc<HttpClient>,
    /// Vulnerability scanning
    osv_client: Arc<OsvClient>,
    vuln_cache: Arc<VulnerabilityCache>,
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
            crates_io: Arc::new(CratesIoRegistry::with_client(Arc::clone(&http_client))),
            npm_registry,
            pypi: Arc::new(PyPiRegistry::with_client(Arc::clone(&http_client))),
            go_proxy: Arc::new(GoProxyRegistry::with_client(Arc::clone(&http_client))),
            packagist: Arc::new(PackagistRegistry::with_client(Arc::clone(&http_client))),
            pub_dev: Arc::new(PubDevRegistry::with_client(Arc::clone(&http_client))),
            nuget: Arc::new(NuGetRegistry::with_client(Arc::clone(&http_client))),
            rubygems: Arc::new(RubyGemsRegistry::with_client(Arc::clone(&http_client))),
            http_client,
            osv_client: Arc::new(OsvClient::default()),
            vuln_cache: Arc::new(VulnerabilityCache::new()),
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
            crates_io: Arc::clone(&self.crates_io),
            npm_registry: Arc::clone(&self.npm_registry),
            pypi: Arc::clone(&self.pypi),
            go_proxy: Arc::clone(&self.go_proxy),
            packagist: Arc::clone(&self.packagist),
            pub_dev: Arc::clone(&self.pub_dev),
            nuget: Arc::clone(&self.nuget),
            rubygems: Arc::clone(&self.rubygems),
            osv_client: Arc::clone(&self.osv_client),
            vuln_cache: Arc::clone(&self.vuln_cache),
        }
    }

    async fn get_version_info(
        &self,
        file_type: FileType,
        package_name: &str,
    ) -> Option<VersionInfo> {
        let cache_key = file_type.cache_key(package_name);

        // Check cache first
        if let Some(cached) = self.version_cache.get(&cache_key) {
            return Some(cached);
        }

        // Fetch from appropriate registry
        let result = match file_type {
            FileType::Cargo => self.crates_io.get_version_info(package_name).await,
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
    ) {
        use crate::vulnerabilities::cache::VulnCacheKey;

        let ecosystem = file_type.to_ecosystem();

        // Build queries for packages not in vulnerability cache
        let queries: Vec<VulnerabilityQuery> = dependencies
            .iter()
            .filter(|dep| {
                let vuln_key = VulnCacheKey::new(ecosystem, &dep.name, &dep.version);
                !vuln_cache.contains(&vuln_key)
            })
            .map(|dep| VulnerabilityQuery {
                ecosystem,
                package_name: dep.name.clone(),
                version: dep.version.clone(),
            })
            .collect();

        if queries.is_empty() {
            tracing::debug!("Background: All vulnerability info cached, skipping OSV query");
            return;
        }

        tracing::info!(
            "Background: Querying OSV.dev for {} packages",
            queries.len()
        );

        // Batch query OSV.dev
        match osv_client.query_batch(&queries).await {
            Ok(results) => {
                let mut updated_count = 0;

                // Update vulnerability cache and version_cache with results
                for (query, result) in queries.iter().zip(results.iter()) {
                    // Mark this package as queried in vuln_cache
                    let vuln_key =
                        VulnCacheKey::new(ecosystem, &query.package_name, &query.version);
                    vuln_cache.insert(vuln_key);

                    // Store vulnerabilities and deprecated status in version_cache
                    let cache_key = file_type.cache_key(&query.package_name);
                    if let Some(mut info) = cache.get(&cache_key) {
                        info.vulnerabilities = result.vulnerabilities.clone();
                        info.deprecated = result.deprecated;
                        if result.deprecated {
                            tracing::info!(
                                "Background: Package {} {} is deprecated (unmaintained)",
                                query.package_name,
                                query.version
                            );
                        }
                        tracing::debug!(
                            "Background: Updated {} {} with {} vulnerabilities, deprecated={}",
                            query.package_name,
                            query.version,
                            result.vulnerabilities.len(),
                            result.deprecated
                        );
                        cache.insert(cache_key, info);
                        updated_count += 1;
                    } else {
                        tracing::warn!(
                            "Background: Could not update vulnerabilities for {}: not found in version cache",
                            query.package_name
                        );
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
            let cache_key = file_type.cache_key(&dep.name);
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

#[tower_lsp::async_trait]
impl LanguageServer for DependiBackend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Parse configuration from initialization options
        let config = Config::from_init_options(params.initialization_options);
        tracing::info!("Configuration: {:?}", config);

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
                let cache_key = file_type.cache_key(&dep.name);
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
            |name| file_type.cache_key(name),
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
                file_type.cache_key(name)
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
