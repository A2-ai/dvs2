//! DVS Server - HTTP API for remote DVS storage access.
//!
//! This crate provides a RESTful HTTP server for accessing DVS storage remotely.

pub mod api;
pub mod auth;
pub mod storage;
pub mod config;

pub use api::create_router;
pub use auth::AuthConfig;
pub use storage::StorageBackend;
pub use config::ServerConfig;

/// Server error types.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Authentication error.
    #[error("authentication failed: {0}")]
    AuthError(String),

    /// Authorization error.
    #[error("not authorized: {0}")]
    NotAuthorized(String),

    /// Storage error.
    #[error("storage error: {0}")]
    StorageError(String),

    /// File not found.
    #[error("file not found: {0}")]
    NotFound(String),

    /// DVS core operation failed.
    #[error("dvs error: {0}")]
    DvsError(#[from] dvs_core::DvsError),

    /// IO error.
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    /// Configuration error.
    #[error("config error: {0}")]
    ConfigError(String),
}

/// Start the HTTP server with the given configuration.
pub async fn start_server(_config: ServerConfig) -> Result<(), ServerError> {
    todo!("Start the HTTP server")
}
