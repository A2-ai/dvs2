//! HTTP API request handling.
//!
//! Provides routing and handlers for CAS (Content-Addressable Storage) endpoints:
//! - `HEAD /objects/{algo}/{hash}` - Check if object exists
//! - `GET /objects/{algo}/{hash}` - Download object
//! - `PUT /objects/{algo}/{hash}` - Upload object
//! - `GET /health` - Health check
//! - `GET /status` - Server status

use std::sync::Arc;
use std::time::Instant;
use fs_err as fs;
use tiny_http::{Request, Response, StatusCode, Header};

use crate::{ServerError, ServerConfig};
use crate::storage::{LocalStorage, StorageBackend, parse_oid};

/// Application state shared across handlers.
pub struct AppState {
    /// Server configuration.
    pub config: Arc<ServerConfig>,
    /// Storage backend.
    pub storage: Arc<LocalStorage>,
    /// Server start time for uptime calculation.
    pub start_time: Instant,
}

impl AppState {
    /// Create a new application state.
    pub fn new(config: ServerConfig) -> Result<Self, ServerError> {
        let storage = LocalStorage::new(config.storage_root.clone())?;
        Ok(Self {
            config: Arc::new(config),
            storage: Arc::new(storage),
            start_time: Instant::now(),
        })
    }
}

/// Handle an incoming HTTP request.
///
/// Routes the request to the appropriate handler based on method and path.
pub fn handle_request(state: &AppState, mut request: Request) -> Result<(), ServerError> {
    let method = request.method().to_string();
    let url = request.url().to_string();

    // Route based on path
    let response = match (method.as_str(), url.as_str()) {
        // Health check
        ("GET", "/health") => handle_health(),

        // Server status
        ("GET", "/status") => handle_status(state),

        // CAS object operations
        (method, path) if path.starts_with("/objects/") => {
            handle_object_request(state, method, path, &mut request)
        }

        // 404 for unknown routes
        _ => Ok(Response::from_string("Not Found")
            .with_status_code(StatusCode(404))),
    };

    // Send response
    match response {
        Ok(resp) => request.respond(resp).map_err(ServerError::IoError),
        Err(e) => {
            let error_response = error_to_response(&e);
            request.respond(error_response).map_err(ServerError::IoError)
        }
    }
}

/// Parse object path: /objects/{algo}/{hash}
fn parse_object_path(path: &str) -> Option<(&str, &str)> {
    let path = path.strip_prefix("/objects/")?;
    let (algo, hash) = path.split_once('/')?;
    // Strip query string if present
    let hash = hash.split('?').next()?;
    Some((algo, hash))
}

/// Handle requests to /objects/{algo}/{hash}
fn handle_object_request(
    state: &AppState,
    method: &str,
    path: &str,
    request: &mut Request,
) -> Result<Response<std::io::Cursor<Vec<u8>>>, ServerError> {
    let (algo, hash) = parse_object_path(path)
        .ok_or_else(|| ServerError::NotFound("invalid object path".to_string()))?;

    let oid = parse_oid(algo, hash)?;

    match method {
        "HEAD" => {
            // Check if object exists
            if state.storage.exists(&oid)? {
                let obj_path = state.storage.get_path(&oid)?;
                let metadata = fs::metadata(&obj_path).map_err(|e| {
                    ServerError::StorageError(format!("failed to get metadata: {e}"))
                })?;

                Ok(Response::from_data(vec![])
                    .with_status_code(StatusCode(200))
                    .with_header(content_length_header(metadata.len())))
            } else {
                Ok(Response::from_data(vec![])
                    .with_status_code(StatusCode(404)))
            }
        }

        "GET" => {
            // Download object
            let data = state.storage.get(&oid)?;
            Ok(Response::from_data(data)
                .with_header(content_type_header("application/octet-stream")))
        }

        "PUT" => {
            // Upload object
            let already_exists = state.storage.exists(&oid)?;

            // Read request body
            let mut body = Vec::new();
            request.as_reader().read_to_end(&mut body)?;

            state.storage.put(&oid, &body)?;

            if already_exists {
                Ok(Response::from_data(vec![])
                    .with_status_code(StatusCode(200)))
            } else {
                Ok(Response::from_data(vec![])
                    .with_status_code(StatusCode(201)))
            }
        }

        _ => Ok(Response::from_string("Method Not Allowed")
            .with_status_code(StatusCode(405))),
    }
}

/// Handle GET /health
fn handle_health() -> Result<Response<std::io::Cursor<Vec<u8>>>, ServerError> {
    let body = serde_json::json!({ "status": "ok" });
    Ok(json_response(200, &body))
}

/// Handle GET /status
fn handle_status(state: &AppState) -> Result<Response<std::io::Cursor<Vec<u8>>>, ServerError> {
    let stats = state.storage.stats()?;

    let body = serde_json::json!({
        "version": dvs_core::VERSION_STRING,
        "storage_used": stats.bytes_used,
        "object_count": stats.object_count,
        "uptime_secs": state.start_time.elapsed().as_secs(),
    });

    Ok(json_response(200, &body))
}

// ============================================================================
// Response Helpers
// ============================================================================

/// Create a JSON response.
fn json_response(status: u16, body: &serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    Response::from_string(json)
        .with_status_code(StatusCode(status))
        .with_header(content_type_header("application/json"))
}

/// Create Content-Type header.
fn content_type_header(content_type: &str) -> Header {
    Header::from_bytes("Content-Type", content_type).unwrap()
}

/// Create Content-Length header.
fn content_length_header(length: u64) -> Header {
    Header::from_bytes("Content-Length", length.to_string()).unwrap()
}

/// Convert ServerError to HTTP response.
fn error_to_response(error: &ServerError) -> Response<std::io::Cursor<Vec<u8>>> {
    let (status, message) = match error {
        ServerError::NotFound(_) => (404, error.to_string()),
        ServerError::AuthError(_) => (401, error.to_string()),
        ServerError::NotAuthorized(_) => (403, error.to_string()),
        ServerError::StorageError(_) => (500, error.to_string()),
        ServerError::DvsError(_) => (500, error.to_string()),
        ServerError::IoError(_) => (500, error.to_string()),
        ServerError::ConfigError(_) => (500, error.to_string()),
    };

    let body = serde_json::json!({ "error": message });
    json_response(status, &body)
}

// ============================================================================
// Response Types (kept for compatibility)
// ============================================================================

/// Response for health check.
#[derive(Debug, serde::Serialize)]
pub struct HealthResponse {
    /// Health status.
    pub status: String,
}

/// Response for server status.
#[derive(Debug, serde::Serialize)]
pub struct StatusResponse {
    /// Server version.
    pub version: String,
    /// Storage usage in bytes.
    pub storage_used: u64,
    /// Number of objects stored.
    pub object_count: u64,
    /// Uptime in seconds.
    pub uptime_secs: u64,
}
