use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use tower_lsp::{LspService, Server};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use dependi_lsp::backend::DependiBackend;

#[derive(Parser)]
#[command(name = "dependi-lsp")]
#[command(about = "Language server for dependency management", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RegistryType {
    Crates,
    Npm,
    Pypi,
    Go,
    Packagist,
    PubDev,
    Nuget,
    Rubygems,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the LSP server (default behavior)
    Lsp,
    /// Scan a file for vulnerabilities and exit with code 1 if found
    Scan {
        /// Path to the dependency file to scan
        #[arg(short, long)]
        file: PathBuf,

        /// Output format: json, markdown, or summary
        #[arg(short, long, default_value = "summary")]
        output: String,

        /// Minimum severity level to report (low, medium, high, critical)
        #[arg(short, long, default_value = "low")]
        min_severity: String,

        /// Exit with code 1 if vulnerabilities are found
        #[arg(long, default_value = "true")]
        fail_on_vulns: bool,
    },
    /// Profile dependency file parsing (for use with cargo-flamegraph)
    ProfileParse {
        /// Path to the dependency file to parse
        #[arg(short, long)]
        file: PathBuf,

        /// Number of iterations (for meaningful profiling)
        #[arg(short, long, default_value = "1000")]
        iterations: usize,
    },
    /// Profile registry requests (for use with cargo-flamegraph)
    ProfileRegistry {
        /// Registry type to profile
        #[arg(short, long)]
        registry: RegistryType,

        /// Packages to fetch (comma-separated)
        #[arg(short, long)]
        packages: String,

        /// Number of iterations (for meaningful profiling)
        #[arg(short, long, default_value = "10")]
        iterations: usize,
    },
    /// Profile full document processing workflow (for use with cargo-flamegraph)
    ProfileFull {
        /// Path to the dependency file to process
        #[arg(short, long)]
        file: PathBuf,

        /// Number of iterations (for meaningful profiling)
        #[arg(short, long, default_value = "10")]
        iterations: usize,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    match cli.command {
        Some(Commands::Scan {
            file,
            output,
            min_severity,
            fail_on_vulns,
        }) => run_scan(file, output, min_severity, fail_on_vulns).await,
        Some(Commands::ProfileParse { file, iterations }) => {
            run_profile_parse(file, iterations).await
        }
        Some(Commands::ProfileRegistry {
            registry,
            packages,
            iterations,
        }) => run_profile_registry(registry, packages, iterations).await,
        Some(Commands::ProfileFull { file, iterations }) => {
            run_profile_full(file, iterations).await
        }
        Some(Commands::Lsp) | None => {
            run_lsp().await;
            ExitCode::SUCCESS
        }
    }
}

async fn run_lsp() {
    tracing::info!("Starting Dependi LSP server");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(DependiBackend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

async fn run_scan(
    file: PathBuf,
    output: String,
    min_severity: String,
    fail_on_vulns: bool,
) -> ExitCode {
    use dependi_lsp::parsers::{
        Parser, cargo::CargoParser, csharp::CsharpParser, dart::DartParser, go::GoParser,
        npm::NpmParser, php::PhpParser, python::PythonParser,
    };
    use dependi_lsp::registries::VulnerabilitySeverity;
    use dependi_lsp::vulnerabilities::{Ecosystem, VulnerabilityQuery, osv::OsvClient};

    // Read file (using async I/O)
    let content = match tokio::fs::read_to_string(&file).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Detect file type and parse
    let (dependencies, ecosystem) = if file_name == "Cargo.toml" {
        (CargoParser::new().parse(&content), Ecosystem::CratesIo)
    } else if file_name == "package.json" {
        (NpmParser::new().parse(&content), Ecosystem::Npm)
    } else if file_name == "requirements.txt" || file_name == "pyproject.toml" {
        (PythonParser::new().parse(&content), Ecosystem::PyPI)
    } else if file_name == "go.mod" {
        (GoParser::new().parse(&content), Ecosystem::Go)
    } else if file_name == "composer.json" {
        (PhpParser::new().parse(&content), Ecosystem::Packagist)
    } else if file_name == "pubspec.yaml" {
        (DartParser::new().parse(&content), Ecosystem::Pub)
    } else if file_name.ends_with(".csproj") {
        (CsharpParser::new().parse(&content), Ecosystem::NuGet)
    } else {
        eprintln!("Unsupported file type: {}", file_name);
        return ExitCode::FAILURE;
    };

    if dependencies.is_empty() {
        println!("No dependencies found in {}", file.display());
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "Scanning {} dependencies in {}...",
        dependencies.len(),
        file.display()
    );

    // Build vulnerability queries
    let queries: Vec<VulnerabilityQuery> = dependencies
        .iter()
        .map(|dep| VulnerabilityQuery {
            ecosystem,
            package_name: dep.name.clone(),
            version: dep.version.clone(),
        })
        .collect();

    // Query OSV.dev
    let osv_client = OsvClient::default();
    let results = match osv_client.query_batch(&queries).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error querying OSV.dev: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Parse minimum severity using shared method
    let min_sev = VulnerabilitySeverity::from_str_loose(&min_severity);

    // Filter and collect vulnerabilities
    let mut total_vulns = 0;
    let mut critical_count = 0;
    let mut high_count = 0;
    let mut medium_count = 0;
    let mut low_count = 0;
    let mut vuln_details: Vec<serde_json::Value> = Vec::new();

    for (dep, result) in dependencies.iter().zip(results.iter()) {
        for vuln in &result.vulnerabilities {
            // Filter by severity using shared method
            if !vuln.severity.meets_threshold(&min_sev) {
                continue;
            }

            total_vulns += 1;
            match vuln.severity {
                VulnerabilitySeverity::Critical => critical_count += 1,
                VulnerabilitySeverity::High => high_count += 1,
                VulnerabilitySeverity::Medium => medium_count += 1,
                VulnerabilitySeverity::Low => low_count += 1,
            }

            vuln_details.push(serde_json::json!({
                "package": dep.name,
                "version": dep.version,
                "id": vuln.id,
                "severity": vuln.severity.as_str(),
                "description": vuln.description,
                "url": vuln.url
            }));
        }
    }

    // Output results
    match output.as_str() {
        "json" => {
            let report = serde_json::json!({
                "file": file.display().to_string(),
                "summary": {
                    "total": total_vulns,
                    "critical": critical_count,
                    "high": high_count,
                    "medium": medium_count,
                    "low": low_count
                },
                "vulnerabilities": vuln_details
            });
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("Failed to serialize report: {}", e),
            }
        }
        "markdown" => {
            println!("# Vulnerability Report\n");
            println!("**File**: {}", file.display());
            println!("**Date**: {}\n", chrono::Local::now().format("%Y-%m-%d"));
            println!("## Summary\n");
            println!("| Severity | Count |");
            println!("|----------|-------|");
            println!("| ⚠ Critical | {} |", critical_count);
            println!("| ▲ High | {} |", high_count);
            println!("| ● Medium | {} |", medium_count);
            println!("| ○ Low | {} |", low_count);
            println!("| **Total** | **{}** |\n", total_vulns);

            if !vuln_details.is_empty() {
                println!("## Vulnerabilities\n");
                for vuln in &vuln_details {
                    let severity_icon = match vuln["severity"].as_str().unwrap_or("low") {
                        "critical" => "⚠",
                        "high" => "▲",
                        "medium" => "●",
                        _ => "○",
                    };
                    println!(
                        "### {}@{}\n",
                        vuln["package"].as_str().unwrap_or(""),
                        vuln["version"].as_str().unwrap_or("")
                    );
                    if let Some(url) = vuln["url"].as_str() {
                        println!(
                            "- **[{}]({})** ({} {}): {}",
                            vuln["id"].as_str().unwrap_or(""),
                            url,
                            severity_icon,
                            vuln["severity"].as_str().unwrap_or("").to_uppercase(),
                            vuln["description"].as_str().unwrap_or("")
                        );
                    } else {
                        println!(
                            "- **{}** ({} {}): {}",
                            vuln["id"].as_str().unwrap_or(""),
                            severity_icon,
                            vuln["severity"].as_str().unwrap_or("").to_uppercase(),
                            vuln["description"].as_str().unwrap_or("")
                        );
                    }
                    println!();
                }
            }
        }
        _ => {
            // Summary format
            println!("Vulnerability Scan Results for {}\n", file.display());
            println!("  ⚠ Critical: {}", critical_count);
            println!("  ▲ High:     {}", high_count);
            println!("  ● Medium:   {}", medium_count);
            println!("  ○ Low:      {}", low_count);
            println!("  ─────────────");
            println!("  Total:      {}\n", total_vulns);

            if total_vulns == 0 {
                println!("[OK] No vulnerabilities found!");
            } else {
                println!("⚠ {} vulnerabilities found!", total_vulns);
            }
        }
    }

    // Exit code
    if fail_on_vulns && total_vulns > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_profile_parse(file: PathBuf, iterations: usize) -> ExitCode {
    use dependi_lsp::parsers::{
        Parser, cargo::CargoParser, csharp::CsharpParser, dart::DartParser, go::GoParser,
        npm::NpmParser, php::PhpParser, python::PythonParser, ruby::RubyParser,
    };

    let content = match tokio::fs::read_to_string(&file).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");

    eprintln!("Profiling parse operations for: {}", file.display());
    eprintln!("Iterations: {}", iterations);
    eprintln!("File size: {} bytes", content.len());

    let start = Instant::now();

    for _ in 0..iterations {
        if file_name.ends_with("Cargo.toml")
            || (file_name.ends_with(".toml") && file_name.contains("cargo"))
        {
            let parser = CargoParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("package.json")
            || (file_name.ends_with(".json") && file_name.contains("package"))
        {
            let parser = NpmParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("requirements.txt") || file_name.ends_with("pyproject.toml") {
            let parser = PythonParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("go.mod") {
            let parser = GoParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("composer.json") {
            let parser = PhpParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("pubspec.yaml") {
            let parser = DartParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with(".csproj") {
            let parser = CsharpParser::new();
            std::hint::black_box(parser.parse(&content));
        } else if file_name.ends_with("Gemfile") {
            let parser = RubyParser::new();
            std::hint::black_box(parser.parse(&content));
        } else {
            eprintln!("Unsupported file type: {}", file_name);
            return ExitCode::FAILURE;
        }
    }

    let elapsed = start.elapsed();
    eprintln!("\nProfiling complete!");
    eprintln!("Total time: {:?}", elapsed);
    eprintln!("Average per iteration: {:?}", elapsed / iterations as u32);

    ExitCode::SUCCESS
}

async fn run_profile_registry(
    registry: RegistryType,
    packages: String,
    iterations: usize,
) -> ExitCode {
    use dependi_lsp::config::NpmRegistryConfig;
    use dependi_lsp::registries::{
        Registry, crates_io::CratesIoRegistry, go_proxy::GoProxyRegistry, npm::NpmRegistry,
        nuget::NuGetRegistry, packagist::PackagistRegistry, pub_dev::PubDevRegistry,
        pypi::PyPiRegistry, rubygems::RubyGemsRegistry,
    };

    let package_list: Vec<&str> = packages.split(',').map(|s| s.trim()).collect();

    eprintln!("Profiling registry requests for: {:?}", registry);
    eprintln!("Packages: {:?}", package_list);
    eprintln!("Iterations: {}", iterations);

    let start = Instant::now();

    for i in 0..iterations {
        eprintln!("Iteration {}/{}", i + 1, iterations);
        for pkg in &package_list {
            let result = match registry {
                RegistryType::Crates => {
                    let reg = CratesIoRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::Npm => {
                    let reg = NpmRegistry::with_client_and_config(
                        dependi_lsp::registries::http_client::create_shared_client()
                            .expect("Failed to create HTTP client"),
                        &NpmRegistryConfig::default(),
                    );
                    reg.get_version_info(pkg).await
                }
                RegistryType::Pypi => {
                    let reg = PyPiRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::Go => {
                    let reg = GoProxyRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::Packagist => {
                    let reg = PackagistRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::PubDev => {
                    let reg = PubDevRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::Nuget => {
                    let reg = NuGetRegistry::default();
                    reg.get_version_info(pkg).await
                }
                RegistryType::Rubygems => {
                    let reg = RubyGemsRegistry::default();
                    reg.get_version_info(pkg).await
                }
            };

            match result {
                Ok(info) => eprintln!(
                    "  {} -> latest: {:?}",
                    pkg,
                    info.latest.as_deref().unwrap_or("N/A")
                ),
                Err(e) => eprintln!("  {} -> error: {}", pkg, e),
            }
        }
    }

    let elapsed = start.elapsed();
    eprintln!("\nProfiling complete!");
    eprintln!("Total time: {:?}", elapsed);
    eprintln!(
        "Average per package fetch: {:?}",
        elapsed / (iterations * package_list.len()) as u32
    );

    ExitCode::SUCCESS
}

async fn run_profile_full(file: PathBuf, iterations: usize) -> ExitCode {
    use dependi_lsp::config::NpmRegistryConfig;
    use dependi_lsp::parsers::{
        Parser, cargo::CargoParser, csharp::CsharpParser, dart::DartParser, go::GoParser,
        npm::NpmParser, php::PhpParser, python::PythonParser, ruby::RubyParser,
    };
    use dependi_lsp::registries::{
        Registry, crates_io::CratesIoRegistry, go_proxy::GoProxyRegistry, npm::NpmRegistry,
        nuget::NuGetRegistry, packagist::PackagistRegistry, pub_dev::PubDevRegistry,
        pypi::PyPiRegistry, rubygems::RubyGemsRegistry,
    };
    use dependi_lsp::vulnerabilities::{Ecosystem, VulnerabilityQuery, osv::OsvClient};
    use futures::future::join_all;

    let content = match tokio::fs::read_to_string(&file).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");

    eprintln!("Profiling full workflow for: {}", file.display());
    eprintln!("Iterations: {}", iterations);

    let start = Instant::now();

    for i in 0..iterations {
        eprintln!("Iteration {}/{}", i + 1, iterations);

        // Step 1: Parse
        let parse_start = Instant::now();
        let (dependencies, ecosystem) = if file_name.ends_with("Cargo.toml")
            || (file_name.ends_with(".toml") && file_name.contains("cargo"))
        {
            (CargoParser::new().parse(&content), Ecosystem::CratesIo)
        } else if file_name.ends_with("package.json")
            || (file_name.ends_with(".json") && file_name.contains("package"))
        {
            (NpmParser::new().parse(&content), Ecosystem::Npm)
        } else if file_name.ends_with("requirements.txt") || file_name.ends_with("pyproject.toml") {
            (PythonParser::new().parse(&content), Ecosystem::PyPI)
        } else if file_name.ends_with("go.mod") {
            (GoParser::new().parse(&content), Ecosystem::Go)
        } else if file_name.ends_with("composer.json") {
            (PhpParser::new().parse(&content), Ecosystem::Packagist)
        } else if file_name.ends_with("pubspec.yaml") {
            (DartParser::new().parse(&content), Ecosystem::Pub)
        } else if file_name.ends_with(".csproj") {
            (CsharpParser::new().parse(&content), Ecosystem::NuGet)
        } else if file_name.ends_with("Gemfile") {
            (RubyParser::new().parse(&content), Ecosystem::RubyGems)
        } else {
            eprintln!("Unsupported file type: {}", file_name);
            return ExitCode::FAILURE;
        };
        let parse_elapsed = parse_start.elapsed();

        if dependencies.is_empty() {
            eprintln!("No dependencies found");
            continue;
        }

        eprintln!("  Parse: {:?} ({} deps)", parse_elapsed, dependencies.len());

        // Step 2: Fetch registry info (parallel, limited to first 10 for profiling)
        let registry_start = Instant::now();
        let deps_to_fetch: Vec<_> = dependencies.iter().take(10).collect();
        let http_client = dependi_lsp::registries::http_client::create_shared_client()
            .expect("Failed to create HTTP client");

        let futures: Vec<_> = deps_to_fetch
            .iter()
            .map(|dep| {
                let name = dep.name.clone();
                let client = http_client.clone();
                async move {
                    match ecosystem {
                        Ecosystem::CratesIo => {
                            let reg = CratesIoRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::Npm => {
                            let reg = NpmRegistry::with_client_and_config(
                                client,
                                &NpmRegistryConfig::default(),
                            );
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::PyPI => {
                            let reg = PyPiRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::Go => {
                            let reg = GoProxyRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::Packagist => {
                            let reg = PackagistRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::Pub => {
                            let reg = PubDevRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::NuGet => {
                            let reg = NuGetRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                        Ecosystem::RubyGems => {
                            let reg = RubyGemsRegistry::with_client(client);
                            reg.get_version_info(&name).await
                        }
                    }
                }
            })
            .collect();

        let _results = join_all(futures).await;
        let registry_elapsed = registry_start.elapsed();
        eprintln!("  Registry: {:?}", registry_elapsed);

        // Step 3: Vulnerability check
        let vuln_start = Instant::now();
        let queries: Vec<VulnerabilityQuery> = deps_to_fetch
            .iter()
            .map(|dep| VulnerabilityQuery {
                ecosystem,
                package_name: dep.name.clone(),
                version: dep.version.clone(),
            })
            .collect();

        let osv_client = OsvClient::default();
        let _ = osv_client.query_batch(&queries).await;
        let vuln_elapsed = vuln_start.elapsed();
        eprintln!("  Vulns: {:?}", vuln_elapsed);
    }

    let elapsed = start.elapsed();
    eprintln!("\nProfiling complete!");
    eprintln!("Total time: {:?}", elapsed);
    eprintln!("Average per iteration: {:?}", elapsed / iterations as u32);

    ExitCode::SUCCESS
}
