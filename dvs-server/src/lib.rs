//! DVS Server - HTTP API for remote DVS storage access.
//!
//! This crate provides a RESTful HTTP server for accessing DVS storage remotely.
//!
//! ## CAS Endpoints
//!
//! The server implements Content-Addressable Storage (CAS) endpoints:
//!
//! - `HEAD /objects/{algo}/{hash}` - Check if object exists (returns 200 or 404)
//! - `GET /objects/{algo}/{hash}` - Download object bytes
//! - `PUT /objects/{algo}/{hash}` - Upload object bytes
//!
//! Where `algo` is the hash algorithm (blake3, sha256, xxh3) and `hash` is the
//! hex-encoded hash value.
//!
//! ## Example
//!
//! ```rust,no_run
//! use dvs_server::{ServerConfig, start_server};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = ServerConfig::default();
//!     start_server(config).await.unwrap();
//! }
//! ```

pub mod api;
pub mod auth;
pub mod storage;
pub mod config;

pub use api::{create_router, AppState};
pub use auth::{AuthConfig, ApiKey, Permission, AuthContext};
pub use storage::{StorageBackend, LocalStorage, StorageStats};
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

    /// Object not found.
    #[error("not found: {0}")]
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
///
/// This will bind to the configured host:port and serve the CAS API.
/// The function runs until the server is shut down.
pub async fn start_server(config: ServerConfig) -> Result<(), ServerError> {
    let bind_addr = config.bind_address();

    let state = AppState::new(config)?;
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| ServerError::ConfigError(format!("failed to bind to {bind_addr}: {e}")))?;

    tracing::info!("DVS server listening on {}", bind_addr);

    axum::serve(listener, app)
        .await
        .map_err(ServerError::IoError)?;

    Ok(())
}
