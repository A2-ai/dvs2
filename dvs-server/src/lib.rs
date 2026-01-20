//! DVS Server - HTTP API for remote DVS storage access.
//!
//! This crate provides a lightweight HTTP server for accessing DVS storage remotely.
//! Uses `tiny_http` for minimal dependencies.
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
//! let config = ServerConfig::default();
//! start_server(config).unwrap();
//! ```

pub mod api;
pub mod auth;
pub mod config;
pub mod storage;

pub use api::AppState;
pub use auth::{
    extract_auth_from_header, require_auth_from_header, require_permission_from_header, ApiKey,
    AuthConfig, AuthContext, Permission,
};
pub use config::ServerConfig;
pub use storage::{LocalStorage, StorageBackend, StorageStats};

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
/// The function runs until the server is shut down (blocking).
pub fn start_server(config: ServerConfig) -> Result<(), ServerError> {
    let bind_addr = config.bind_address();

    let state = AppState::new(config)?;

    let server = tiny_http::Server::http(&bind_addr)
        .map_err(|e| ServerError::ConfigError(format!("failed to bind to {bind_addr}: {e}")))?;

    eprintln!("DVS server listening on {}", bind_addr);

    // Handle requests in a loop
    for request in server.incoming_requests() {
        if let Err(e) = api::handle_request(&state, request) {
            eprintln!("Error handling request: {}", e);
        }
    }

    Ok(())
}

/// Start the server and return a handle for shutdown.
///
/// This is useful for testing - returns the server and a function to stop it.
pub fn start_server_background(
    config: ServerConfig,
) -> Result<(String, ServerHandle), ServerError> {
    let bind_addr = config.bind_address();
    let state = AppState::new(config)?;

    let server = tiny_http::Server::http(&bind_addr)
        .map_err(|e| ServerError::ConfigError(format!("failed to bind to {bind_addr}: {e}")))?;

    let url = format!("http://{}", bind_addr);

    Ok((url, ServerHandle { server, state }))
}

/// Handle to a running server.
pub struct ServerHandle {
    server: tiny_http::Server,
    state: AppState,
}

impl ServerHandle {
    /// Process a single request (for testing).
    pub fn handle_one(&self) -> Result<bool, ServerError> {
        match self.server.try_recv() {
            Ok(Some(request)) => {
                api::handle_request(&self.state, request)?;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(ServerError::IoError(std::io::Error::other(e.to_string()))),
        }
    }

    /// Get the server URL.
    pub fn url(&self) -> String {
        format!("http://{}", self.server.server_addr())
    }
}
