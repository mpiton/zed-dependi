//! Configuration management for Dependi LSP

use serde::Deserialize;

/// Default cache TTL (1 hour)
const DEFAULT_CACHE_TTL_SECS: u64 = 3600;

/// LSP configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Inlay hints configuration
    pub inlay_hints: InlayHintsConfig,
    /// Diagnostics configuration
    pub diagnostics: DiagnosticsConfig,
    /// Cache configuration
    pub cache: CacheConfig,
    /// Packages to ignore (glob patterns)
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// Inlay hints configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InlayHintsConfig {
    /// Enable inlay hints
    pub enabled: bool,
    /// Show hints for up-to-date packages
    pub show_up_to_date: bool,
}

impl Default for InlayHintsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_up_to_date: true,
        }
    }
}

/// Diagnostics configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DiagnosticsConfig {
    /// Enable diagnostics
    pub enabled: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Cache configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Cache TTL in seconds
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl_secs: DEFAULT_CACHE_TTL_SECS,
        }
    }
}

impl Config {
    /// Parse configuration from initialization options
    pub fn from_init_options(options: Option<serde_json::Value>) -> Self {
        match options {
            Some(value) => serde_json::from_value(value).unwrap_or_default(),
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.inlay_hints.enabled);
        assert!(config.inlay_hints.show_up_to_date);
        assert!(config.diagnostics.enabled);
        assert_eq!(config.cache.ttl_secs, DEFAULT_CACHE_TTL_SECS);
        assert!(config.ignore.is_empty());
    }

    #[test]
    fn test_parse_from_json() {
        let json = json!({
            "inlay_hints": {
                "enabled": false,
                "show_up_to_date": false
            },
            "diagnostics": {
                "enabled": false
            },
            "cache": {
                "ttl_secs": 7200
            },
            "ignore": ["test-*", "internal-pkg"]
        });

        let config = Config::from_init_options(Some(json));
        assert!(!config.inlay_hints.enabled);
        assert!(!config.inlay_hints.show_up_to_date);
        assert!(!config.diagnostics.enabled);
        assert_eq!(config.cache.ttl_secs, 7200);
        assert_eq!(config.ignore.len(), 2);
    }

    #[test]
    fn test_partial_config() {
        let json = json!({
            "inlay_hints": {
                "enabled": false
            }
        });

        let config = Config::from_init_options(Some(json));
        assert!(!config.inlay_hints.enabled);
        // Other fields should use defaults
        assert!(config.inlay_hints.show_up_to_date);
        assert!(config.diagnostics.enabled);
    }
}
