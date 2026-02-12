//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.
use std::path::PathBuf;

use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use miniextendr_api::{
    list, miniextendr, miniextendr_module, r_println, AsSerializeRow, DataFrame, List, Missing,
};

use anyhow::{anyhow, bail, Result};

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{add_files, get_files, get_status, AddResult, Compression, FileStatus, GetResult};

#[miniextendr]
pub fn dvs_init(
    #[miniextendr(default = r#"".""#)] path: PathBuf,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
    #[miniextendr(default = "NULL")] permissions: Option<String>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "FALSE")] no_compression: bool,
) -> Result<List> {
    let mut config = Config::new_local(&path, permissions, group)?;

    if no_compression {
        config.set_compression(Compression::None);
    }
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }

    init(&path, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

#[miniextendr]
pub fn dvs_add(
    paths: Vec<PathBuf>,
    glob: Missing<Option<String>>,
    message: Missing<Option<String>>,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let message = if message.is_missing() {
        None
    } else {
        message.unwrap()
    };
    let glob = if glob.is_missing() {
        None
    } else {
        glob.unwrap()
    };

    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;
    let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
        .into_iter()
        .collect();

    if all_paths.is_empty() {
        bail!("No files to add")
    }

    Ok(DataFrame::from_iter(
        add_files(
            all_paths,
            &dvs_paths,
            config.backend(),
            message,
            config.compression(),
        )?
        .into_iter()
        .map(|x| x.into()),
    ))
}

#[miniextendr]
pub fn dvs_status() -> Result<DataFrame<AsSerializeRow<FileStatus>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let statuses = get_status(&paths)?;

    Ok(DataFrame::from_iter(statuses.into_iter().map(|x| x.into())))
}

#[miniextendr]
pub fn dvs_get(
    paths: Vec<PathBuf>,
    glob: Missing<Option<String>>,
) -> Result<DataFrame<AsSerializeRow<GetResult>>> {
    let current_dir = std::env::current_dir()?;

    let glob = if glob.is_missing() {
        None
    } else {
        glob.unwrap()
    };

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let all_paths: Vec<_> = resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths)?
        .into_iter()
        .collect();
    if all_paths.is_empty() {
        bail!("No files to get")
    }

    let results = get_files(all_paths, &dvs_paths, config.backend())?;

    Ok(DataFrame::from_iter(results.into_iter().map(|x| x.into())))
}

miniextendr_module! {
    mod dvs;
    fn dvs_init;
    fn dvs_add;
    fn dvs_status;
    fn dvs_get;
}
