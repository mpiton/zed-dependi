//! RustSec Advisory Database client for Rust-specific vulnerability data
//!
//! This module provides integration with the RustSec advisory database
//! for more detailed Rust-specific vulnerability information.
//!
//! NOTE: Currently disabled due to rustsec crate API changes.
//! OSV.dev already aggregates RustSec data, so this is optional.
//! TODO: Update to use the new rustsec API.

use crate::registries::Vulnerability;

/// RustSec advisory database client (currently a stub)
///
/// The rustsec crate API has changed significantly. For now, we rely on
/// OSV.dev which aggregates RustSec advisories. This client can be
/// implemented later for additional Rust-specific details like affected functions.
pub struct RustSecClient {
    _enabled: bool,
}

impl RustSecClient {
    /// Create a new RustSec client
    pub fn new() -> Self {
        Self { _enabled: false }
    }

    /// Query vulnerabilities for a Rust crate
    ///
    /// Currently returns empty - use OSV.dev for Rust vulnerability data.
    pub async fn query(&self, _crate_name: &str, _version: &str) -> anyhow::Result<Vec<Vulnerability>> {
        // TODO: Implement using rustsec crate when API is stabilized
        // For now, OSV.dev covers RustSec advisories
        Ok(vec![])
    }
}

impl Default for RustSecClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rustsec_client_returns_empty() {
        let client = RustSecClient::new();
        let result = client.query("serde", "1.0.0").await.unwrap();
        assert!(result.is_empty());
    }
}
