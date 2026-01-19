//! HTTP API routes and handlers.

use axum::{Router, extract::State, response::IntoResponse, Json};
use std::sync::Arc;
use crate::{ServerError, config::ServerConfig};

/// Application state shared across handlers.
#[derive(Clone)]
pub struct AppState {
    /// Server configuration.
    pub config: Arc<ServerConfig>,
}

/// Create the API router with all routes.
pub fn create_router(_state: AppState) -> Router {
    todo!("Create API router with routes")
}

// ============================================================================
// File Operations
// ============================================================================

/// GET /api/v1/files/:hash - Download a file by hash.
pub async fn get_file(
    State(_state): State<AppState>,
    _hash: axum::extract::Path<String>,
) -> Result<axum::body::Bytes, ServerError> {
    todo!("Download file by hash")
}

/// POST /api/v1/files - Upload a file.
pub async fn upload_file(
    State(_state): State<AppState>,
    _body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, ServerError> {
    todo!("Upload file")
}

/// HEAD /api/v1/files/:hash - Check if file exists.
pub async fn check_file(
    State(_state): State<AppState>,
    _hash: axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, ServerError> {
    todo!("Check if file exists")
}

/// DELETE /api/v1/files/:hash - Delete a file (admin only).
pub async fn delete_file(
    State(_state): State<AppState>,
    _hash: axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, ServerError> {
    todo!("Delete file")
}

// ============================================================================
// Metadata Operations
// ============================================================================

/// GET /api/v1/metadata/:hash - Get metadata for a file.
pub async fn get_metadata(
    State(_state): State<AppState>,
    _hash: axum::extract::Path<String>,
) -> Result<Json<dvs_core::Metadata>, ServerError> {
    todo!("Get file metadata")
}

/// POST /api/v1/metadata - Upload metadata.
pub async fn upload_metadata(
    State(_state): State<AppState>,
    Json(_metadata): Json<dvs_core::Metadata>,
) -> Result<axum::http::StatusCode, ServerError> {
    todo!("Upload metadata")
}

// ============================================================================
// Health and Status
// ============================================================================

/// GET /api/v1/health - Health check endpoint.
pub async fn health_check() -> impl IntoResponse {
    Json(HealthResponse { status: "ok".to_string() })
}

/// GET /api/v1/status - Server status.
pub async fn server_status(
    State(_state): State<AppState>,
) -> Result<Json<StatusResponse>, ServerError> {
    todo!("Get server status")
}

// ============================================================================
// Response Types
// ============================================================================

/// Response for file upload.
#[derive(Debug, serde::Serialize)]
pub struct UploadResponse {
    /// Blake3 hash of the uploaded file.
    pub hash: String,
    /// Size of the file in bytes.
    pub size: u64,
}

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
    /// Number of files stored.
    pub file_count: u64,
    /// Uptime in seconds.
    pub uptime_secs: u64,
}
