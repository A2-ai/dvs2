//! dvsR: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use miniextendr_api::{miniextendr, miniextendr_module, r_error};
use serde::Serialize;
use std::path::PathBuf;

// Re-export dvs-core types for internal use
use dvs_core::{add, get, init, status, DvsError, FileStatus, Outcome};

// =============================================================================
// Helper: Convert DvsError to R error
// =============================================================================

/// Convert a DvsError to an R error with structured information.
fn dvs_error_to_r(e: DvsError) -> ! {
    let error_type = e.error_type();
    let message = e.to_string();
    r_error!("[{}] {}", error_type, message);
}

// =============================================================================
// JSON-serializable result types
// =============================================================================

#[derive(Serialize)]
struct InitResult {
    storage_dir: String,
    permissions: Option<String>,
    group: Option<String>,
    hash_algo: String,
    metadata_format: String,
}

#[derive(Serialize)]
struct AddResultJson {
    path: String,
    outcome: String,
    size: f64,
    checksum: String,
    error: Option<String>,
    error_message: Option<String>,
}

#[derive(Serialize)]
struct GetResultJson {
    path: String,
    outcome: String,
    size: f64,
    checksum: String,
    error: Option<String>,
    error_message: Option<String>,
}

#[derive(Serialize)]
struct StatusResultJson {
    path: String,
    status: String,
    size: f64,
    checksum: String,
    add_time: Option<String>,
    saved_by: Option<String>,
    message: Option<String>,
    error: Option<String>,
    error_message: Option<String>,
}

// =============================================================================
// Basic Information
// =============================================================================

/// @title Hello DVS
/// @description A simple test function to verify the Rust bindings work.
/// @param name A character string with the user's name.
/// @return A greeting string.
/// @examples
/// dvs_hello("World")
/// @export
#[miniextendr]
pub fn dvs_hello(name: &str) -> String {
    format!("Hello, {}! DVS is ready.", name)
}

/// @title DVS Version
/// @description Returns the version of the DVS Rust backend.
/// @return A character string with the version.
/// @examples
/// dvs_version()
/// @export
#[miniextendr]
pub fn dvs_version() -> String {
    dvs_core::version_string().to_string()
}

// =============================================================================
// Core Operations (return JSON)
// =============================================================================

/// @title Initialize DVS (internal)
/// @description Internal function returning JSON. Use dvs_init() wrapper.
/// @param storage_dir Character string specifying the path to the storage directory.
/// @param permissions Optional integer specifying file permissions (octal, e.g., 420 for 0644).
/// @param group Optional character string specifying the Unix group for stored files.
/// @return A JSON string with initialization details.
/// @keywords internal
#[miniextendr]
pub fn dvs_init_json(
    storage_dir: &str,
    permissions: Option<i32>,
    group: Option<&str>,
) -> String {
    let storage_path = PathBuf::from(storage_dir);
    let perms = permissions.map(|p| p as u32);

    match init(&storage_path, perms, group) {
        Ok(config) => {
            let result = InitResult {
                storage_dir: config.storage_dir.display().to_string(),
                permissions: config.permissions.map(|p| format!("{:o}", p)),
                group: config.group,
                hash_algo: format!("{:?}", config.hash_algo),
                metadata_format: format!("{:?}", config.metadata_format),
            };
            serde_json::to_string(&result).unwrap_or_else(|e| {
                r_error!("JSON serialization failed: {}", e);
            })
        }
        Err(e) => dvs_error_to_r(e),
    }
}

/// @title Add Files to DVS (internal)
/// @description Internal function returning JSON. Use dvs_add() wrapper.
/// @param files Character vector of file paths or glob patterns to add.
/// @param message Optional character string describing this version.
/// @return A JSON array of results.
/// @keywords internal
#[miniextendr]
pub fn dvs_add_json(files: Vec<String>, message: Option<&str>) -> String {
    let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();

    match add(&paths, message) {
        Ok(results) => {
            let json_results: Vec<AddResultJson> = results
                .into_iter()
                .map(|r| AddResultJson {
                    path: r.relative_path.display().to_string(),
                    outcome: outcome_to_string(r.outcome),
                    size: r.size as f64,
                    checksum: r.blake3_checksum,
                    error: r.error,
                    error_message: r.error_message,
                })
                .collect();
            serde_json::to_string(&json_results).unwrap_or_else(|e| {
                r_error!("JSON serialization failed: {}", e);
            })
        }
        Err(e) => dvs_error_to_r(e),
    }
}

/// @title Get Files from DVS Storage (internal)
/// @description Internal function returning JSON. Use dvs_get() wrapper.
/// @param files Character vector of file paths or glob patterns to retrieve.
/// @return A JSON array of results.
/// @keywords internal
#[miniextendr]
pub fn dvs_get_json(files: Vec<String>) -> String {
    let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();

    match get(&paths) {
        Ok(results) => {
            let json_results: Vec<GetResultJson> = results
                .into_iter()
                .map(|r| GetResultJson {
                    path: r.relative_path.display().to_string(),
                    outcome: outcome_to_string(r.outcome),
                    size: r.size as f64,
                    checksum: r.blake3_checksum,
                    error: r.error,
                    error_message: r.error_message,
                })
                .collect();
            serde_json::to_string(&json_results).unwrap_or_else(|e| {
                r_error!("JSON serialization failed: {}", e);
            })
        }
        Err(e) => dvs_error_to_r(e),
    }
}

/// @title Check DVS File Status (internal)
/// @description Internal function returning JSON. Use dvs_status() wrapper.
/// @param files Character vector of file paths or glob patterns to check.
/// @return A JSON array of results.
/// @keywords internal
#[miniextendr]
pub fn dvs_status_json(files: Vec<String>) -> String {
    let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();

    match status(&paths) {
        Ok(results) => {
            let json_results: Vec<StatusResultJson> = results
                .into_iter()
                .map(|r| StatusResultJson {
                    path: r.relative_path.display().to_string(),
                    status: file_status_to_string(r.status),
                    size: r.size as f64,
                    checksum: r.blake3_checksum,
                    add_time: r.add_time.map(|t| t.to_rfc3339()),
                    saved_by: r.saved_by,
                    message: r.message,
                    error: r.error,
                    error_message: r.error_message,
                })
                .collect();
            serde_json::to_string(&json_results).unwrap_or_else(|e| {
                r_error!("JSON serialization failed: {}", e);
            })
        }
        Err(e) => dvs_error_to_r(e),
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn outcome_to_string(outcome: Outcome) -> String {
    match outcome {
        Outcome::Copied => "copied".to_string(),
        Outcome::Present => "present".to_string(),
        Outcome::Error => "error".to_string(),
    }
}

fn file_status_to_string(status: FileStatus) -> String {
    match status {
        FileStatus::Current => "current".to_string(),
        FileStatus::Absent => "absent".to_string(),
        FileStatus::Unsynced => "unsynced".to_string(),
        FileStatus::Error => "error".to_string(),
    }
}

// =============================================================================
// Module Registration
// =============================================================================

miniextendr_module! {
    mod dvs;

    // Basic info
    fn dvs_hello;
    fn dvs_version;

    // Core operations (JSON)
    fn dvs_init_json;
    fn dvs_add_json;
    fn dvs_get_json;
    fn dvs_status_json;
}
