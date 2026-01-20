//! Server configuration.

use crate::{auth::AuthConfig, ServerError};
use fs_err as fs;
use std::path::PathBuf;

/// Configuration for the DVS server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Storage root directory.
    pub storage_root: PathBuf,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Maximum upload size in bytes.
    pub max_upload_size: u64,
    /// Enable CORS.
    pub cors_enabled: bool,
    /// Allowed CORS origins.
    pub cors_origins: Vec<String>,
    /// Log level.
    pub log_level: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            storage_root: PathBuf::from("/var/dvs/storage"),
            auth: AuthConfig::default(),
            max_upload_size: 100 * 1024 * 1024, // 100MB
            cors_enabled: false,
            cors_origins: vec![],
            log_level: "info".to_string(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from file.
    pub fn load(path: &std::path::Path) -> Result<Self, ServerError> {
        let contents = fs::read_to_string(path)
            .map_err(|e| ServerError::ConfigError(format!("failed to read config: {e}")))?;
        let config: Self = toml::from_str(&contents)
            .map_err(|e| ServerError::ConfigError(format!("failed to parse config: {e}")))?;
        Ok(config)
    }

    /// Save configuration to file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), ServerError> {
        let contents = toml::to_string_pretty(self)
            .map_err(|e| ServerError::ConfigError(format!("failed to serialize config: {e}")))?;
        fs::write(path, contents)
            .map_err(|e| ServerError::ConfigError(format!("failed to write config: {e}")))?;
        Ok(())
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), ServerError> {
        // Check port is valid (non-zero)
        if self.port == 0 {
            return Err(ServerError::ConfigError("port cannot be 0".to_string()));
        }

        // Check host is not empty
        if self.host.is_empty() {
            return Err(ServerError::ConfigError("host cannot be empty".to_string()));
        }

        // Check storage_root exists if it's an absolute path
        if self.storage_root.is_absolute() && !self.storage_root.exists() {
            return Err(ServerError::ConfigError(format!(
                "storage_root does not exist: {}",
                self.storage_root.display()
            )));
        }

        // Check max_upload_size is reasonable (at least 1KB)
        if self.max_upload_size < 1024 {
            return Err(ServerError::ConfigError(
                "max_upload_size must be at least 1024 bytes".to_string(),
            ));
        }

        Ok(())
    }

    /// Get the bind address as a string.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_config_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");

        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            storage_root: PathBuf::from("/tmp/storage"),
            max_upload_size: 50 * 1024 * 1024,
            cors_enabled: true,
            cors_origins: vec!["http://localhost:3000".to_string()],
            ..Default::default()
        };

        config.save(&path).unwrap();
        let loaded = ServerConfig::load(&path).unwrap();

        assert_eq!(loaded.host, "0.0.0.0");
        assert_eq!(loaded.port, 3000);
        assert_eq!(loaded.max_upload_size, 50 * 1024 * 1024);
        assert!(loaded.cors_enabled);
        assert_eq!(loaded.cors_origins.len(), 1);
    }

    #[test]
    fn test_config_validate_valid() {
        let dir = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            storage_root: dir.path().to_path_buf(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validate_invalid_port() {
        let config = ServerConfig {
            port: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_empty_host() {
        let config = ServerConfig {
            host: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_small_upload_size() {
        let config = ServerConfig {
            max_upload_size: 100,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_load_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "this is not valid toml {{{{").unwrap();

        assert!(ServerConfig::load(&path).is_err());
    }
}
