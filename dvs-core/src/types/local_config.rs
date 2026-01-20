//! Local configuration for `.dvs/config.toml`.
//!
//! This config stores user-specific settings that shouldn't be committed
//! to version control, such as authentication tokens and default remotes.

use crate::DvsError;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Local configuration stored in `.dvs/config.toml`.
///
/// This is separate from the repository config (`dvs.toml`/`dvs.yaml`)
/// and contains user-specific settings like auth tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Default remote URL for push/pull operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Authentication settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,

    /// Cache settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheConfig>,
}

/// Authentication configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Bearer token for authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Cache configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum cache size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,
}

impl LocalConfig {
    /// Create a new empty local config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load config from a TOML file.
    ///
    /// Returns default config if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, DvsError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        let config: LocalConfig = toml::from_str(&contents)
            .map_err(|e| DvsError::config_error(format!("failed to parse local config: {e}")))?;
        Ok(config)
    }

    /// Save config to a TOML file.
    pub fn save(&self, path: &Path) -> Result<(), DvsError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self).map_err(|e| {
            DvsError::config_error(format!("failed to serialize local config: {e}"))
        })?;
        fs::write(path, contents)?;
        Ok(())
    }

    /// Get the base URL if configured.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// Set the base URL.
    pub fn set_base_url(&mut self, url: Option<String>) {
        self.base_url = url;
    }

    /// Get the auth token if configured.
    pub fn auth_token(&self) -> Option<&str> {
        self.auth.as_ref().and_then(|a| a.token.as_deref())
    }

    /// Set the auth token.
    pub fn set_auth_token(&mut self, token: Option<String>) {
        if let Some(token) = token {
            let auth = self.auth.get_or_insert_with(AuthConfig::default);
            auth.token = Some(token);
        } else if let Some(auth) = &mut self.auth {
            auth.token = None;
            // Clean up empty auth section
            if auth.token.is_none() {
                self.auth = None;
            }
        }
    }

    /// Get the max cache size if configured.
    pub fn cache_max_size(&self) -> Option<u64> {
        self.cache.as_ref().and_then(|c| c.max_size)
    }

    /// Set the max cache size.
    pub fn set_cache_max_size(&mut self, size: Option<u64>) {
        if let Some(size) = size {
            let cache = self.cache.get_or_insert_with(CacheConfig::default);
            cache.max_size = Some(size);
        } else if let Some(cache) = &mut self.cache {
            cache.max_size = None;
            // Clean up empty cache section
            if cache.max_size.is_none() {
                self.cache = None;
            }
        }
    }

    /// Check if the config is empty (all fields are None/default).
    pub fn is_empty(&self) -> bool {
        self.base_url.is_none() && self.auth.is_none() && self.cache.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_config_default() {
        let config = LocalConfig::new();
        assert!(config.is_empty());
        assert!(config.base_url().is_none());
        assert!(config.auth_token().is_none());
        assert!(config.cache_max_size().is_none());
    }

    #[test]
    fn test_local_config_setters() {
        let mut config = LocalConfig::new();

        config.set_base_url(Some("https://example.com".to_string()));
        assert_eq!(config.base_url(), Some("https://example.com"));

        config.set_auth_token(Some("secret-token".to_string()));
        assert_eq!(config.auth_token(), Some("secret-token"));

        config.set_cache_max_size(Some(1024 * 1024 * 100)); // 100MB
        assert_eq!(config.cache_max_size(), Some(104857600));

        assert!(!config.is_empty());
    }

    #[test]
    fn test_local_config_clear() {
        let mut config = LocalConfig::new();
        config.set_base_url(Some("https://example.com".to_string()));
        config.set_auth_token(Some("token".to_string()));

        config.set_base_url(None);
        config.set_auth_token(None);

        assert!(config.base_url().is_none());
        assert!(config.auth_token().is_none());
    }

    #[test]
    fn test_local_config_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let mut config = LocalConfig::new();
        config.set_base_url(Some("https://dvs.example.com".to_string()));
        config.set_auth_token(Some("my-secret-token".to_string()));
        config.set_cache_max_size(Some(1024 * 1024 * 500)); // 500MB

        config.save(&config_path).unwrap();

        let loaded = LocalConfig::load(&config_path).unwrap();
        assert_eq!(loaded.base_url(), Some("https://dvs.example.com"));
        assert_eq!(loaded.auth_token(), Some("my-secret-token"));
        assert_eq!(loaded.cache_max_size(), Some(524288000));
    }

    #[test]
    fn test_local_config_load_nonexistent() {
        let config = LocalConfig::load(Path::new("/nonexistent/config.toml")).unwrap();
        assert!(config.is_empty());
    }

    #[test]
    fn test_local_config_toml_format() {
        let mut config = LocalConfig::new();
        config.set_base_url(Some("https://example.com".to_string()));
        config.set_auth_token(Some("token123".to_string()));

        let toml_str = toml::to_string_pretty(&config).unwrap();

        // Verify TOML structure
        assert!(toml_str.contains("base_url = \"https://example.com\""));
        assert!(toml_str.contains("[auth]"));
        assert!(toml_str.contains("token = \"token123\""));
    }
}
