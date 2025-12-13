use std::sync::Arc;

use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::cache::MemoryCache;
use crate::parsers::cargo::CargoParser;
use crate::parsers::go::GoParser;
use crate::parsers::npm::NpmParser;
use crate::parsers::php::PhpParser;
use crate::parsers::python::PythonParser;
use crate::parsers::{Dependency, Parser};
use crate::providers::code_actions::create_code_actions;
use crate::providers::completion::get_completions;
use crate::providers::diagnostics::create_diagnostics;
use crate::providers::inlay_hints::create_inlay_hint;
use crate::registries::crates_io::CratesIoRegistry;
use crate::registries::go_proxy::GoProxyRegistry;
use crate::registries::npm::NpmRegistry;
use crate::registries::packagist::PackagistRegistry;
use crate::registries::pypi::PyPiRegistry;
use crate::registries::{Registry, VersionInfo};

/// File type for determining which parser/registry to use
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    Cargo,
    Npm,
    Python,
    Go,
    Php,
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
    /// Cache for documents and their parsed state
    documents: DashMap<Url, DocumentState>,
    /// Cache for version information (keyed by "registry:package")
    version_cache: Arc<MemoryCache>,
    /// Parsers
    cargo_parser: CargoParser,
    npm_parser: NpmParser,
    python_parser: PythonParser,
    go_parser: GoParser,
    php_parser: PhpParser,
    /// Registry clients
    crates_io: Arc<CratesIoRegistry>,
    npm_registry: Arc<NpmRegistry>,
    pypi: Arc<PyPiRegistry>,
    go_proxy: Arc<GoProxyRegistry>,
    packagist: Arc<PackagistRegistry>,
}

impl DependiBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: DashMap::new(),
            version_cache: Arc::new(MemoryCache::new()),
            cargo_parser: CargoParser::new(),
            npm_parser: NpmParser::new(),
            python_parser: PythonParser::new(),
            go_parser: GoParser::new(),
            php_parser: PhpParser::new(),
            crates_io: Arc::new(CratesIoRegistry::default()),
            npm_registry: Arc::new(NpmRegistry::default()),
            pypi: Arc::new(PyPiRegistry::default()),
            go_proxy: Arc::new(GoProxyRegistry::default()),
            packagist: Arc::new(PackagistRegistry::default()),
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
        } else {
            None
        }
    }

    /// Check if a document is a supported dependency file
    fn is_supported_file(uri: &Url) -> bool {
        Self::detect_file_type(uri).is_some()
    }

    /// Parse a document and extract dependencies
    fn parse_document(&self, uri: &Url, content: &str) -> Vec<Dependency> {
        match Self::detect_file_type(uri) {
            Some(FileType::Cargo) => self.cargo_parser.parse(content),
            Some(FileType::Npm) => self.npm_parser.parse(content),
            Some(FileType::Python) => self.python_parser.parse(content),
            Some(FileType::Go) => self.go_parser.parse(content),
            Some(FileType::Php) => self.php_parser.parse(content),
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
        }
    }

    /// Fetch version info for a package (with caching)
    async fn get_version_info(&self, file_type: FileType, package_name: &str) -> Option<VersionInfo> {
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

        // Store document state
        self.documents.insert(
            uri.clone(),
            DocumentState {
                dependencies: dependencies.clone(),
                file_type,
            },
        );

        // Publish diagnostics
        let diagnostics = create_diagnostics(&dependencies, &self.version_cache, |name| {
            Self::cache_key(file_type, name)
        });

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;

        // Refresh inlay hints
        self.client
            .send_request::<request::InlayHintRefreshRequest>(())
            .await
            .ok();
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for DependiBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
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
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Dependi LSP initialized")
            .await;
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
        self.client
            .publish_diagnostics(uri, vec![], None)
            .await;
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
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
            .map(|dep| {
                let cache_key = Self::cache_key(file_type, &dep.name);
                let version_info = self.version_cache.get(&cache_key);
                create_inlay_hint(dep, version_info.as_ref())
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

                parts.push(format!("**Current:** {}", dep.version));
                if let Some(latest) = &info.latest {
                    parts.push(format!("**Latest:** {}", latest));
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
        let completions = get_completions(
            &doc.dependencies,
            position,
            &self.version_cache,
            |name| Self::cache_key(file_type, name),
        );

        match completions {
            Some(items) => Ok(Some(CompletionResponse::Array(items))),
            None => Ok(Some(CompletionResponse::Array(vec![]))),
        }
    }
}
