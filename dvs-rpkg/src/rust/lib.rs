//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use miniextendr_api::{miniextendr, miniextendr_module};
use std::path::PathBuf;

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::init::init;
use dvs::paths::{find_repo_root, DvsPaths};
use dvs::{add_files, get_file, get_file_status, get_files, get_status};

/// Initialize a DVS repository with a local backend.
///
/// # Arguments
/// * `directory` - Directory to initialize (must be inside a git repo)
/// * `storage_path` - Path where file content will be stored
/// * `permissions` - Optional Unix permissions for storage files (e.g., "755")
/// * `group` - Optional Unix group for storage files
///
/// # Returns
/// Empty string on success, or error message on failure.
#[miniextendr]
pub fn dvs_init(
    directory: &str,
    storage_path: &str,
    permissions: Option<String>,
    group: Option<String>,
) -> String {
    let config = match Config::new_local(storage_path, permissions, group) {
        Ok(c) => c,
        Err(e) => return format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    };

    match init(directory, config) {
        Ok(()) => "{}".to_string(),
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Find DVS configuration from a directory.
///
/// # Arguments
/// * `directory` - Directory to start searching from
///
/// # Returns
/// JSON with config info if found, null if not found, or error message.
#[miniextendr]
pub fn dvs_find_config(directory: &str) -> String {
    match Config::find(directory) {
        Some(Ok(_config)) => {
            // Config found - return success indicator
            // We can't easily serialize the full config, so just confirm it exists
            "{\"found\": true}".to_string()
        }
        Some(Err(e)) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        None => "{\"found\": false}".to_string(),
    }
}

/// Find the repository root from a given directory.
///
/// # Arguments
/// * `start_dir` - Directory to start searching from
///
/// # Returns
/// JSON with the repo root path, or null if not found.
#[miniextendr]
pub fn dvs_find_repo_root(start_dir: &str) -> String {
    match find_repo_root(start_dir) {
        Some(path) => format!(
            "{{\"repo_root\": \"{}\"}}",
            escape_json(&path.display().to_string())
        ),
        None => "{\"repo_root\": null}".to_string(),
    }
}

/// Add files matching a glob pattern to DVS.
///
/// # Arguments
/// * `pattern` - Glob pattern to match files (e.g., "*.csv", "data/**/*.parquet")
/// * `cwd` - Current working directory for pattern matching
/// * `repo_root` - Root of the repository
/// * `storage_path` - Path to storage backend
/// * `message` - Optional commit message
/// * `permissions` - Optional Unix permissions for storage files
/// * `group` - Optional Unix group for storage files
///
/// # Returns
/// JSON array of AddResult objects with path and outcome for each file.
#[miniextendr]
pub fn dvs_add_files(
    pattern: &str,
    cwd: &str,
    repo_root: &str,
    storage_path: &str,
    message: Option<String>,
    permissions: Option<String>,
    group: Option<String>,
) -> String {
    let config = match Config::new_local(storage_path, permissions, group) {
        Ok(c) => c,
        Err(e) => return format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    };

    let paths = DvsPaths::new(
        PathBuf::from(cwd),
        PathBuf::from(repo_root),
        config.metadata_folder_name(),
    );

    match add_files(pattern, &paths, config.backend(), message) {
        Ok(results) => match serde_json::to_string(&results) {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        },
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Get files matching a glob pattern from DVS storage.
///
/// # Arguments
/// * `pattern` - Glob pattern to match tracked files
/// * `cwd` - Current working directory for pattern matching
/// * `repo_root` - Root of the repository
/// * `storage_path` - Path to storage backend
/// * `permissions` - Optional Unix permissions for storage files
/// * `group` - Optional Unix group for storage files
///
/// # Returns
/// JSON array of GetResult objects with path and outcome for each file.
#[miniextendr]
pub fn dvs_get_files(
    pattern: &str,
    cwd: &str,
    repo_root: &str,
    storage_path: &str,
    permissions: Option<String>,
    group: Option<String>,
) -> String {
    let config = match Config::new_local(storage_path, permissions, group) {
        Ok(c) => c,
        Err(e) => return format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    };

    let paths = DvsPaths::new(
        PathBuf::from(cwd),
        PathBuf::from(repo_root),
        config.metadata_folder_name(),
    );

    match get_files(pattern, &paths, config.backend()) {
        Ok(results) => match serde_json::to_string(&results) {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        },
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Get a single file from DVS storage.
///
/// # Arguments
/// * `relative_path` - Path to the file relative to repo root
/// * `cwd` - Current working directory
/// * `repo_root` - Root of the repository
/// * `storage_path` - Path to storage backend
/// * `permissions` - Optional Unix permissions for storage files
/// * `group` - Optional Unix group for storage files
///
/// # Returns
/// JSON with outcome ("copied" or "present").
#[miniextendr]
pub fn dvs_get_file(
    relative_path: &str,
    cwd: &str,
    repo_root: &str,
    storage_path: &str,
    permissions: Option<String>,
    group: Option<String>,
) -> String {
    let config = match Config::new_local(storage_path, permissions, group) {
        Ok(c) => c,
        Err(e) => return format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    };

    let paths = DvsPaths::new(
        PathBuf::from(cwd),
        PathBuf::from(repo_root),
        config.metadata_folder_name(),
    );

    match get_file(config.backend(), &paths, relative_path) {
        Ok(outcome) => match serde_json::to_string(&outcome) {
            Ok(json) => format!("{{\"outcome\": {}}}", json),
            Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        },
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Get status of all tracked files.
///
/// # Arguments
/// * `cwd` - Current working directory
/// * `repo_root` - Root of the repository
/// * `metadata_folder_name` - Name of the metadata folder (default ".dvs")
///
/// # Returns
/// JSON array of FileStatus objects with path and status for each tracked file.
#[miniextendr]
pub fn dvs_get_status(cwd: &str, repo_root: &str, metadata_folder_name: &str) -> String {
    let paths = DvsPaths::new(
        PathBuf::from(cwd),
        PathBuf::from(repo_root),
        metadata_folder_name,
    );

    match get_status(&paths) {
        Ok(results) => match serde_json::to_string(&results) {
            Ok(json) => json,
            Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        },
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Get status of a single file.
///
/// # Arguments
/// * `relative_path` - Path to the file relative to repo root
/// * `cwd` - Current working directory
/// * `repo_root` - Root of the repository
/// * `metadata_folder_name` - Name of the metadata folder (default ".dvs")
///
/// # Returns
/// JSON with status ("untracked", "current", "absent", or "unsynced").
#[miniextendr]
pub fn dvs_get_file_status(
    relative_path: &str,
    cwd: &str,
    repo_root: &str,
    metadata_folder_name: &str,
) -> String {
    let paths = DvsPaths::new(
        PathBuf::from(cwd),
        PathBuf::from(repo_root),
        metadata_folder_name,
    );

    match get_file_status(&paths, relative_path) {
        Ok(status) => match serde_json::to_string(&status) {
            Ok(json) => format!("{{\"status\": {}}}", json),
            Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
        },
        Err(e) => format!("{{\"error\": \"{}\"}}", escape_json(&e.to_string())),
    }
}

/// Helper function to escape strings for JSON.
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

miniextendr_module! {
    mod dvs;
    fn dvs_init;
    fn dvs_find_config;
    fn dvs_find_repo_root;
    fn dvs_add_files;
    fn dvs_get_files;
    fn dvs_get_file;
    fn dvs_get_status;
    fn dvs_get_file_status;
}
