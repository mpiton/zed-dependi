//! RustSec advisory cache (issue #237)
//!
//! Caches the result of `OSV GET /vulns/{id}` to avoid redundant network
//! requests for the same RUSTSEC advisory across LSP sessions.

pub mod memory;
pub mod sqlite;

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Cached classification of a single OSV advisory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AdvisoryKind {
    /// Advisory exists at OSV; we recorded the parts we need.
    Found {
        summary: Option<String>,
        unmaintained: bool,
    },
    /// Advisory ID returned 404 from OSV.
    NotFound,
}

/// One cache entry: an advisory ID plus its classification and fetch time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedAdvisory {
    pub id: String,
    pub kind: AdvisoryKind,
    pub fetched_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_advisory_round_trips_through_json() {
        let advisory = CachedAdvisory {
            id: "RUSTSEC-2020-0036".to_string(),
            kind: AdvisoryKind::Found {
                summary: Some("failure crate is unmaintained".to_string()),
                unmaintained: true,
            },
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&advisory).expect("serialize");
        let back: CachedAdvisory = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(advisory, back);
    }

    #[test]
    fn not_found_kind_round_trips() {
        let advisory = CachedAdvisory {
            id: "RUSTSEC-9999-0001".to_string(),
            kind: AdvisoryKind::NotFound,
            fetched_at: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&advisory).expect("serialize");
        let back: CachedAdvisory = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(advisory, back);
    }
}
