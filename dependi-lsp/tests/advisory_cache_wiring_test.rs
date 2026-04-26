//! Smoke test: HybridAdvisoryCache constructs without panic and survives a
//! cache-miss lookup. Guards Task 26 of the RustSec advisory cache plan
//! (issue #237): the LSP backend must be able to instantiate the cache at
//! startup without exploding even when the SQLite layer is unavailable.

use std::sync::Arc;

use dependi_lsp::cache::{AdvisoryReadCache, HybridAdvisoryCache};

#[tokio::test]
async fn hybrid_advisory_cache_constructs_without_panic() {
    let cache = Arc::new(HybridAdvisoryCache::new());
    assert!(cache.get("RUSTSEC-2020-0036").await.is_none());
}
