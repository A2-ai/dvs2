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
use dvs::{add_files, get_files, get_status, AddResult, FileStatus, GetResult};

#[miniextendr]
pub fn dvs_init(
    #[miniextendr(default = r#"".""#)] directory: PathBuf,
    #[miniextendr(default = "NULL")] permissions: Option<String>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
) -> Result<List> {
    let mut config = Config::new_local(&directory, permissions, group)?;

    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    init(&directory, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

#[miniextendr]
pub fn dvs_add(
    patterns: Vec<PathBuf>,
    message: Missing<Option<String>>,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let message = if message.is_missing() {
        None
    } else {
        message.unwrap()
    };

    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let mut is_valid_paths = paths.validate_for_add(&patterns);

    let valid_paths = is_valid_paths
        .extract_if(.., |(_path, is_valid)| *is_valid)
        .map(|x| x.0)
        .collect();
    let invalid_paths = is_valid_paths;
    let _ = is_valid_paths;

    let all_invalid_paths: Vec<_> = invalid_paths
        .into_iter()
        .map(|x| x.0.display().to_string())
        .collect();
    r_println!(
        "dvs failed to add the following files: {}",
        all_invalid_paths.join(",")
    );

    Ok(DataFrame::from_serialize(add_files(
        valid_paths,
        &paths,
        config.backend(),
        message,
    )?))
}

#[miniextendr]
pub fn dvs_status() -> Result<DataFrame<AsSerializeRow<FileStatus>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let statuses = get_status(&paths)?;

    Ok(DataFrame::from_serialize(statuses))
}

#[miniextendr]
pub fn dvs_get(patterns: Vec<PathBuf>) -> Result<DataFrame<AsSerializeRow<GetResult>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let paths = DvsPaths::from_cwd(&config)?;

    let mut is_valid_paths = paths.validate_for_get(&patterns);

    let valid_paths = is_valid_paths
        .extract_if(.., |(_path, is_valid)| *is_valid)
        .map(|x| x.0)
        .collect();
    let invalid_paths = is_valid_paths;
    let _ = is_valid_paths;

    let all_invalid_paths: Vec<_> = invalid_paths
        .into_iter()
        .map(|x| x.0.display().to_string())
        .collect();
    r_println!(
        "dvs failed to get the following files: {}",
        all_invalid_paths.join(",")
    );

    Ok(DataFrame::from_serialize(get_files(
        valid_paths,
        &paths,
        config.backend(),
    )?))
}

miniextendr_module! {
    mod dvs;
    fn dvs_init;
    fn dvs_add;
    fn dvs_status;
    fn dvs_get;
}
