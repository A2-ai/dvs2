//! Server configuration.

use std::path::PathBuf;
use crate::{ServerError, auth::AuthConfig};

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
    pub fn load(_path: &std::path::Path) -> Result<Self, ServerError> {
        todo!("Load server config from file")
    }

    /// Save configuration to file.
    pub fn save(&self, _path: &std::path::Path) -> Result<(), ServerError> {
        todo!("Save server config to file")
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), ServerError> {
        todo!("Validate server config")
    }

    /// Get the bind address as a string.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
