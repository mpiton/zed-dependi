//! Vulnerability scanning for dependencies
//!
//! This module provides vulnerability detection using OSV.dev API
//! as the primary source for all ecosystems.

pub mod cache;
pub mod osv;

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
