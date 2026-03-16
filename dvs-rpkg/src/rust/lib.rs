//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use std::path::PathBuf;

use miniextendr_api::{AsSerializeRow, DataFrame, List, Missing, list, miniextendr, r_println};

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

#[miniextendr]
pub fn dvs_init(
    storage_path: PathBuf,
    #[miniextendr(default = r#"".""#)] root_dir: PathBuf,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
    #[miniextendr(default = "FALSE")] no_compression: bool,
) -> Result<List> {
    let mut config = Config::new_local(&storage_path, group)?;

    if no_compression {
        config.set_compression(Compression::None);
    }
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    init(&root_dir, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

#[miniextendr]
pub fn dvs_add(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    message: Missing<Option<String>>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "FALSE")] dry_run: bool,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let message = if message.is_missing() {
        None
    } else {
        message.unwrap()
    };

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
            dry_run,
        )?
        .into_iter()
        .map(|x| x.into()),
    ))
}

#[miniextendr]
pub fn dvs_status(
    #[miniextendr(default = "FALSE")] current: bool,
    #[miniextendr(default = "FALSE")] absent: bool,
    #[miniextendr(default = "FALSE")] unsynced: bool,
) -> Result<DataFrame<AsSerializeRow<FileStatus>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let show_all = !current && !absent && !unsynced;
    let mut statuses = get_status(&paths)?;
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

#[miniextendr]
pub fn dvs_get(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "FALSE")] dry_run: bool,
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

    let results = get_files(all_paths, &dvs_paths, config.backend(), dry_run)?;
    Ok(DataFrame::from_iter(results.into_iter().map(|x| x.into())))
}

miniextendr_api::miniextendr_init!(dvs);
