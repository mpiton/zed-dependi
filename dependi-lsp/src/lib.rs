//! Dependi LSP - Language Server for dependency management
//!
//! This crate provides a Language Server Protocol implementation for
//! managing dependencies in various package managers (Cargo, npm, etc.)

pub mod backend;
pub mod cache;
pub mod config;
pub mod parsers;
pub mod providers;
pub mod registries;
pub mod vulnerabilities;
