//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use std::path::PathBuf;

use miniextendr_api::{AsSerializeRow, DataFrame, List, list, miniextendr, r_println};

use anyhow::{Result, anyhow};

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{
    AddResult, Compression, FileStatus, GetResult, Status, StatusDetail, add_files, get_files,
    get_status,
};

/// Initialize a DVS repository in the given directory.
///
/// Creates the `.dvs` metadata folder and configures storage for versioned files.
///
/// @param storage_path Path to the storage directory where file contents are kept.
/// @param root_dir Repository root directory. Defaults to the current working directory.
/// @param group Unix group name to set on stored files for shared access.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @param no_compression If `TRUE`, disable compression for stored files.
#[miniextendr]
pub fn dvs_init(
    storage_path: PathBuf,
    #[miniextendr(default = "NULL")] root_dir: Option<PathBuf>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
    #[miniextendr(default = "NULL")] no_compression: Option<bool>,
) -> Result<List> {
    let root_dir = match root_dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let mut config = Config::new_local(&storage_path, group)?;

    if no_compression == Some(true) {
        config.set_compression(Compression::None);
    }
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    init(&root_dir, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

/// Add files to DVS-managed storage.
///
/// Hashes and copies the specified files into the content-addressable store,
/// replacing each original with a `.dvs` metadata file.
///
/// @param files Character vector of file paths to add.
/// @param message Optional commit message describing why the files were added.
/// @param glob Optional glob pattern to select files (e.g. `"data/*.csv"`).
/// @param dry_run If `TRUE`, report what would be added without modifying anything.
#[miniextendr]
pub fn dvs_add(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] message: Option<String>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let all_paths: Vec<_> = resolve_paths_for_add(files, glob.as_deref(), &paths)?
        .into_iter()
        .collect();
    if all_paths.is_empty() {
        return Err(anyhow!("No files to add"));
    }

    Ok(DataFrame::from_iter(
        add_files(
            all_paths,
            &paths,
            config.backend(),
            message,
            config.compression(),
            dry_run.unwrap_or(false),
            None,
        )?
        .into_iter()
        .map(|x| x.into()),
    ))
}

/// Report the sync status of DVS-managed files.
///
/// Compares `.dvs` metadata files against their stored contents and local
/// working copies. By default all files are shown; pass filter flags to
/// restrict output.
///
/// @param current If `TRUE`, include files whose local copy matches storage.
/// @param absent If `TRUE`, include files that exist in metadata but not locally.
/// @param unsynced If `TRUE`, include files whose local copy differs from storage.
#[miniextendr]
pub fn dvs_status(
    #[miniextendr(default = "NULL")] current: Option<bool>,
    #[miniextendr(default = "NULL")] absent: Option<bool>,
    #[miniextendr(default = "NULL")] unsynced: Option<bool>,
) -> Result<DataFrame<AsSerializeRow<FileStatus>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let current = current.unwrap_or(false);
    let absent = absent.unwrap_or(false);
    let unsynced = unsynced.unwrap_or(false);
    let show_all = !current && !absent && !unsynced;

    let mut statuses = get_status(&paths, None)?;
    if !show_all {
        statuses.retain(|x| match &x.detail {
            StatusDetail::Success { status } => {
                (current && *status == Status::Current)
                    || (absent && *status == Status::Absent)
                    || (unsynced && *status == Status::Unsynced)
            }
            StatusDetail::Error { .. } => true,
        });
    }

    Ok(DataFrame::from_iter(statuses.into_iter().map(|x| x.into())))
}

/// Retrieve files from DVS storage into the working directory.
///
/// Reads `.dvs` metadata files, fetches the corresponding contents from
/// the content-addressable store, and writes them to their original paths.
///
/// @param files Character vector of `.dvs` metadata file paths to retrieve.
/// @param glob Optional glob pattern to select `.dvs` files (e.g. `"data/*.dvs"`).
/// @param dry_run If `TRUE`, report what would be retrieved without writing files.
#[miniextendr]
pub fn dvs_get(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
) -> Result<DataFrame<AsSerializeRow<GetResult>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let all_paths: Vec<_> = resolve_paths_for_get(files, glob.as_deref(), &dvs_paths)?
        .into_iter()
        .collect();
    if all_paths.is_empty() {
        return Err(anyhow!("No files to get"));
    }

    let results = get_files(
        all_paths,
        &dvs_paths,
        config.backend(),
        dry_run.unwrap_or(false),
        None,
    )?;
    Ok(DataFrame::from_iter(results.into_iter().map(|x| x.into())))
}

#[miniextendr]
/// Detect cores for parallelism
///
/// Returns the number of logical cores currently available in the system.
///
/// @note Unlike `parallel::detectCores()`, `num_cpus()` respects cgroups.
pub fn num_cpus() -> usize {
    num_cpus::get()
}

miniextendr_api::miniextendr_init!(dvs);
