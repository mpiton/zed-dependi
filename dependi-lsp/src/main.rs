mod backend;
mod cache;
mod config;
mod parsers;
mod providers;
mod registries;
mod utils;
mod vulnerabilities;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tower_lsp::{LspService, Server};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::backend::DependiBackend;

#[derive(Parser)]
#[command(name = "dependi-lsp")]
#[command(about = "Language server for dependency management", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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
    use crate::parsers::{
        Parser, cargo::CargoParser, csharp::CsharpParser, dart::DartParser, go::GoParser,
        npm::NpmParser, php::PhpParser, python::PythonParser,
    };
    use crate::registries::VulnerabilitySeverity;
    use crate::vulnerabilities::{Ecosystem, VulnerabilityQuery, osv::OsvClient};

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
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
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
