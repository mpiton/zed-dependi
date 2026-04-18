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

pub fn create_shared_client() -> anyhow::Result<Client> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        .build()?;

    Ok(client)
}
