//! HTTP API routes and handlers.
//!
//! Provides CAS (Content-Addressable Storage) endpoints for DVS objects:
//! - `HEAD /objects/{algo}/{hash}` - Check if object exists
//! - `GET /objects/{algo}/{hash}` - Download object
//! - `PUT /objects/{algo}/{hash}` - Upload object

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, head, put},
    Json,
};
use std::sync::Arc;
use std::time::Instant;
use crate::{ServerError, config::ServerConfig};
use crate::storage::{LocalStorage, StorageBackend, parse_oid};

/// Application state shared across handlers.
#[derive(Clone)]
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

/// Create the API router with all routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // CAS object endpoints
        .route("/objects/{algo}/{hash}", head(check_object))
        .route("/objects/{algo}/{hash}", get(get_object))
        .route("/objects/{algo}/{hash}", put(put_object))
        // Health and status
        .route("/health", get(health_check))
        .route("/status", get(server_status))
        .with_state(state)
}

// ============================================================================
// CAS Object Operations
// ============================================================================

/// Path parameters for object endpoints.
#[derive(Debug, serde::Deserialize)]
pub struct ObjectPath {
    /// Hash algorithm (blake3, sha256, xxh3).
    algo: String,
    /// Hex-encoded hash value.
    hash: String,
}

/// HEAD /objects/{algo}/{hash} - Check if object exists.
///
/// Returns 200 OK if exists, 404 Not Found otherwise.
/// Response includes Content-Length header with object size.
pub async fn check_object(
    State(state): State<AppState>,
    Path(params): Path<ObjectPath>,
) -> Result<Response, ServerError> {
    let oid = parse_oid(&params.algo, &params.hash)?;

    if state.storage.exists(&oid)? {
        // Get the object size for Content-Length header
        let path = state.storage.get_path(&oid)?;
        let metadata = std::fs::metadata(&path).map_err(|e| {
            ServerError::StorageError(format!("failed to get metadata: {e}"))
        })?;

        Ok((
            StatusCode::OK,
            [(header::CONTENT_LENGTH, metadata.len().to_string())],
        ).into_response())
    } else {
        Ok(StatusCode::NOT_FOUND.into_response())
    }
}

/// GET /objects/{algo}/{hash} - Download object.
///
/// Returns the raw object bytes with Content-Type: application/octet-stream.
pub async fn get_object(
    State(state): State<AppState>,
    Path(params): Path<ObjectPath>,
) -> Result<Response, ServerError> {
    let oid = parse_oid(&params.algo, &params.hash)?;

    let data = state.storage.get(&oid)?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_LENGTH, &data.len().to_string()),
        ],
        data,
    ).into_response())
}

/// PUT /objects/{algo}/{hash} - Upload object.
///
/// Stores the request body as the object. Idempotent - re-uploading
/// the same content is a no-op.
///
/// Returns 201 Created for new objects, 200 OK if already exists.
pub async fn put_object(
    State(state): State<AppState>,
    Path(params): Path<ObjectPath>,
    body: Bytes,
) -> Result<Response, ServerError> {
    let oid = parse_oid(&params.algo, &params.hash)?;

    let already_exists = state.storage.exists(&oid)?;
    state.storage.put(&oid, &body)?;

    if already_exists {
        Ok(StatusCode::OK.into_response())
    } else {
        Ok(StatusCode::CREATED.into_response())
    }
}

// ============================================================================
// Health and Status
// ============================================================================

/// GET /health - Health check endpoint.
pub async fn health_check() -> impl IntoResponse {
    Json(HealthResponse { status: "ok".to_string() })
}

/// GET /status - Server status.
pub async fn server_status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, ServerError> {
    let stats = state.storage.stats()?;

    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        storage_used: stats.bytes_used,
        object_count: stats.object_count,
        uptime_secs: state.start_time.elapsed().as_secs(),
    }))
}

// ============================================================================
// Response Types
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

// ============================================================================
// Error Handling
// ============================================================================

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ServerError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ServerError::AuthError(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            ServerError::NotAuthorized(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ServerError::StorageError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ServerError::DvsError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ServerError::IoError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ServerError::ConfigError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
