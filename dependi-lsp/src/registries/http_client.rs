//! Shared HTTP client for all registry clients.
//!
//! This module provides a shared HTTP client with proper configuration
//! for connection pooling, HTTP/2, and timeout handling. Sharing a single
//! client across all registries enables:
//!
//! - Connection reuse across different registries
//! - HTTP/2 multiplexing where supported
//! - Reduced TLS handshake overhead
//! - Shared DNS cache
//! - Lower memory footprint

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;

const USER_AGENT: &str = concat!(
    "dependi-lsp/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/mpiton/zed-dependi)"
);

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Creates a shared, Arc-wrapped reqwest Client configured for connection pooling, timeouts, and TCP keepalive.
///
/// On success returns an `Arc<Client>` configured with a custom User-Agent, a default request timeout,
/// a connect timeout, a pool idle timeout, a max of 10 idle connections per host, and a 60s TCP keepalive.
///
/// # Errors
///
/// Returns an error if building the underlying `reqwest::Client` fails.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// let client: Arc<reqwest::Client> = crate::create_shared_client().expect("failed to create client");
/// // `client` can now be cloned and shared between callers:
/// let cloned = Arc::clone(&client);
/// drop(cloned);
/// ```
pub fn create_shared_client() -> anyhow::Result<Arc<Client>> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        .build()?;

    Ok(Arc::new(client))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_shared_client() {
        let client = create_shared_client().expect("Failed to create client");
        assert!(Arc::strong_count(&client) == 1);
    }

    #[test]
    fn test_client_can_be_cloned() {
        let client = create_shared_client().expect("Failed to create client");
        let client2 = Arc::clone(&client);
        assert!(Arc::strong_count(&client) == 2);
        drop(client2);
        assert!(Arc::strong_count(&client) == 1);
    }
}