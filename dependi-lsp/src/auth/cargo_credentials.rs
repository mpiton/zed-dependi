//! Parser for Cargo credentials files.
//!
//! Parses `~/.cargo/credentials.toml` or `$CARGO_HOME/credentials.toml`
//! to extract authentication tokens for alternative Cargo registries.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize)]
struct CargoCredentials {
    #[serde(default)]
    registries: HashMap<String, RegistryCredential>,
}

#[derive(Debug, Deserialize)]
struct RegistryCredential {
    token: Option<String>,
}

/// Parse `.cargo/credentials.toml` file for registry tokens.
///
/// Looks in `$CARGO_HOME/credentials.toml` or `~/.cargo/credentials.toml`.
///
/// # Returns
/// A map of registry name to token string.
pub async fn parse_cargo_credentials() -> HashMap<String, String> {
    let credentials_path = get_credentials_path();

    let Some(path) = credentials_path else {
        return HashMap::new();
    };

    if !path.exists() {
        return HashMap::new();
    }

    let content = match fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    parse_credentials_content(&content)
}

fn get_credentials_path() -> Option<PathBuf> {
    // Try CARGO_HOME first
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let cargo_home_path = PathBuf::from(&cargo_home);
        let path = cargo_home_path.join("credentials.toml");
        if path.exists() {
            return Some(path);
        }
        // Also try without .toml extension (older format)
        let path = cargo_home_path.join("credentials");
        if path.exists() {
            return Some(path);
        }
    }

    // Fall back to ~/.cargo
    if let Some(home) = dirs::home_dir() {
        let cargo_dir = home.join(".cargo");
        let path = cargo_dir.join("credentials.toml");
        if path.exists() {
            return Some(path);
        }
        // Also try without .toml extension
        let path = cargo_dir.join("credentials");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn parse_credentials_content(content: &str) -> HashMap<String, String> {
    let credentials: CargoCredentials = match toml::from_str(content) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    credentials
        .registries
        .into_iter()
        .filter_map(|(name, cred)| cred.token.map(|t| (name, t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_credentials() {
        let content = r#"
[registries.my-registry]
token = "secret_token_123"

[registries.another-registry]
token = "another_secret"
"#;

        let result = parse_credentials_content(content);

        assert_eq!(result.len(), 2);
        assert_eq!(
            result.get("my-registry"),
            Some(&"secret_token_123".to_string())
        );
        assert_eq!(
            result.get("another-registry"),
            Some(&"another_secret".to_string())
        );
    }

    #[test]
    fn test_parse_empty_credentials() {
        let content = "";
        let result = parse_credentials_content(content);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_no_registries() {
        let content = r#"
[net]
git-fetch-with-cli = true
"#;
        let result = parse_credentials_content(content);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_registry_without_token() {
        let content = r#"
[registries.my-registry]
# token not set
"#;
        let result = parse_credentials_content(content);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_mixed_registries() {
        let content = r#"
[registries.with-token]
token = "has_token"

[registries.without-token]
# no token here
"#;
        let result = parse_credentials_content(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("with-token"), Some(&"has_token".to_string()));
    }
}
