//! Authentication support for private registries.
//!
//! This module provides authentication mechanisms for accessing private package registries.
//! Tokens are read from environment variables only - they should NEVER be stored in LSP settings.
//!
//! ## Security
//!
//! - Tokens are read from environment variables at initialization time
//! - Tokens are NEVER logged in any circumstances
//! - All authenticated requests use HTTPS only
//! - Sensitive data is redacted in error messages
//!
//! ## Submodules
//!
//! - [`cargo_credentials`]: Parser for Cargo credentials files (`~/.cargo/credentials.toml`)
//! - [`npmrc`]: Parser for npm configuration files (`.npmrc`)

pub mod cargo_credentials;
pub mod npmrc;

/// Redact a token for safe logging.
///
/// Shows only the first few characters to help identify which token is in use
/// without exposing the full secret.
pub fn redact_token(token: &str) -> String {
    if token.len() <= 4 {
        "****".to_string()
    } else {
        format!("{}...", &token[..4])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_token() {
        assert_eq!(redact_token("abc"), "****");
        assert_eq!(redact_token("abcdefgh"), "abcd...");
        assert_eq!(redact_token("npm_1234567890"), "npm_...");
    }
}
