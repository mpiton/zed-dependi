//! Vulnerability scanning for dependencies
//!
//! This module provides vulnerability detection using:
//! - OSV.dev API (primary source for all ecosystems)
//! - RustSec crate (additional Rust-specific details)

use crate::registries::Vulnerability;

pub mod cache;
pub mod osv;
pub mod rustsec_client;

/// Ecosystem identifiers for vulnerability sources
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    /// Rust crates (crates.io)
    CratesIo,
    /// JavaScript/Node packages (npm)
    Npm,
    /// Python packages (PyPI)
    PyPI,
    /// Go modules
    Go,
    /// PHP packages (Packagist)
    Packagist,
    /// Dart/Flutter packages (pub.dev)
    Pub,
    /// .NET packages (NuGet)
    NuGet,
}

impl Ecosystem {
    /// Convert to OSV.dev ecosystem string
    pub fn as_osv_str(&self) -> &'static str {
        match self {
            Ecosystem::CratesIo => "crates.io",
            Ecosystem::Npm => "npm",
            Ecosystem::PyPI => "PyPI",
            Ecosystem::Go => "Go",
            Ecosystem::Packagist => "Packagist",
            Ecosystem::Pub => "Pub",
            Ecosystem::NuGet => "NuGet",
        }
    }
}

/// Query for vulnerability lookup
#[derive(Debug, Clone)]
pub struct VulnerabilityQuery {
    /// Package name
    pub package_name: String,
    /// Package version (normalized, without ^ or ~ prefixes)
    pub version: String,
    /// Target ecosystem
    pub ecosystem: Ecosystem,
}

/// Trait for vulnerability data sources
#[allow(async_fn_in_trait, dead_code)]
pub trait VulnerabilitySource: Send + Sync {
    /// Query vulnerabilities for a single package
    async fn query(&self, query: &VulnerabilityQuery) -> anyhow::Result<Vec<Vulnerability>>;

    /// Batch query for multiple packages (more efficient)
    async fn query_batch(
        &self,
        queries: &[VulnerabilityQuery],
    ) -> anyhow::Result<Vec<Vec<Vulnerability>>>;
}
