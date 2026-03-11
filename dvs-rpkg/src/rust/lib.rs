//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use std::path::PathBuf;

use miniextendr_api::{
    list, miniextendr, miniextendr_module, r_println, AsSerializeRow, DataFrame, List, Missing,
};

use anyhow::{anyhow, Result};

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{add_files, get_files, get_status, AddResult, FileStatus, GetResult, OutputOptions};

/// Initialize a new DVS repository.
///
/// @param directory Path to the storage directory.
/// @param group Optional Unix group for storage files.
/// @param metadata_folder_name Optional custom metadata folder name (default `.dvs`).
/// @return A list with `status = "initialized"`.
/// @export
#[miniextendr]
pub fn dvs_init(
    #[miniextendr(default = r#"".""#)] directory: PathBuf,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
) -> Result<List> {
    let mut config = Config::new_local(&directory, group)?;

    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    init(&directory, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

/// Add files to DVS storage.
///
/// @param files Character vector of file paths to add.
/// @param message Optional commit message.
/// @param verbose Verbosity level. `FALSE`/`0` = quiet, `TRUE`/`1` = progress,
///   `2` = per-step detail, `3` = detail + CSV timing log written to working directory.
/// @return A data.frame with columns `path`, `outcome`, `hash`, `size` (or `error`).
/// @export
#[miniextendr]
pub fn dvs_add(
    files: Vec<PathBuf>,
    message: Missing<Option<String>>,
    #[miniextendr(default = "0L")] verbose: i32,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let message = if message.is_missing() {
        None
    } else {
        message.unwrap()
    };

    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let output = OutputOptions { dry_run: false, verbosity: verbose.max(0) as u8, ..Default::default() };
    Ok(DataFrame::from_iter(
        add_files(files, &paths, config.backend(), message, config.compression(), &output)?
            .into_iter()
            .map(|x| x.into()),
    ))
}

/// Get the status of all tracked files.
///
/// @param verbose Verbosity level. `FALSE`/`0` = quiet, `TRUE`/`1` = progress,
///   `2` = per-step detail, `3` = detail + CSV timing log written to working directory.
/// @return A data.frame with columns `path` and `status` (or `error`).
/// @export
#[miniextendr]
pub fn dvs_status(
    #[miniextendr(default = "0L")] verbose: i32,
) -> Result<DataFrame<AsSerializeRow<FileStatus>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let statuses = get_status(&paths, &OutputOptions { verbosity: verbose.max(0) as u8, ..Default::default() })?;

    Ok(DataFrame::from_iter(statuses.into_iter().map(|x| x.into())))
}

/// Retrieve files from DVS storage.
///
/// @param files Character vector of file paths to retrieve.
/// @param verbose Verbosity level. `FALSE`/`0` = quiet, `TRUE`/`1` = progress,
///   `2` = per-step detail, `3` = detail + CSV timing log written to working directory.
/// @return A data.frame with columns `path`, `outcome`, `size` (or `error`).
/// @export
#[miniextendr]
pub fn dvs_get(
    files: Vec<PathBuf>,
    #[miniextendr(default = "0L")] verbose: i32,
) -> Result<DataFrame<AsSerializeRow<GetResult>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let results = get_files(files, &paths, config.backend(), &OutputOptions { verbosity: verbose.max(0) as u8, ..Default::default() })?;
    Ok(DataFrame::from_iter(results.into_iter().map(|x| x.into())))
}

miniextendr_module! {
    mod dvs;
    fn dvs_init;
    fn dvs_add;
    fn dvs_status;
    fn dvs_get;
}
