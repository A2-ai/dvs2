//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use miniextendr_api::serde::AsSerialize;
use miniextendr_api::{miniextendr, miniextendr_module, r_println, List};
use std::path::PathBuf;

use anyhow::{anyhow, Result};

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::init::init;
use dvs::paths::{find_repo_root, DvsPaths};
use dvs::{add_files, get_file, get_file_status, get_files, get_status, AddResult};

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
    #[miniextendr(default = r#"".""#)] directory: &str,
    #[miniextendr(default = "NULL")] permissions: Option<String>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
) -> Result<List> {
    let mut config = Config::new_local(directory, permissions, group)?;

    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    init(directory, config)?;

    r_println!("DVS Initialized");
    Ok(List::from_pairs(vec![("status", "initialized")]))
}

/// Add files matching a glob pattern to DVS.
///
/// # Arguments
/// * `pattern` - Glob pattern to match files (e.g., "*.csv", "data/**/*.parquet")
/// * `message` - Optional commit message
///
///
#[miniextendr]
pub fn dvs_add(pattern: &str, message: Option<String>) -> Result<AsSerialize<Vec<AddResult>>> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    Ok(add_files(pattern, &paths, config.backend(), message)?.into())
}

miniextendr_module! {
    mod dvs;
    fn dvs_init;
    fn dvs_add;
}
