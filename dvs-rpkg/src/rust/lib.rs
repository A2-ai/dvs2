//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use std::collections::HashSet;
use std::path::PathBuf;

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
    #[miniextendr(default = r#"".""#)] directory: PathBuf,
    #[miniextendr(default = "NULL")] permissions: Option<String>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
    #[miniextendr(default = "FALSE")] no_compression: bool,
) -> Result<List> {
    let mut config = Config::new_local(&directory, permissions, group)?;

    if no_compression {
        config.set_compression(Compression::None);
    }
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }

    init(&directory, config)?;

    r_println!("DVS Initialized");
    Ok(list!("status" = "initialized"))
}

/// Resolve paths for `add` command following ripgrep-style behavior:
fn resolve_paths_for_add(paths: Vec<PathBuf>, dvs_paths: &DvsPaths) -> Result<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    let repo_root = dvs_paths.repo_root().canonicalize()?;

    // If no paths given, default to cwd
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    for path in paths {
        let full_path = dvs_paths
            .cwd()
            .join(&path)
            .canonicalize()
            .map_err(|_| anyhow!("Path not found: {}", path.display()))?;

        // Explicit file: we ignore the glob and add it to the file
        if full_path.is_file() {
            // Ensure it's in the repo
            let relative_to_root = full_path
                .strip_prefix(&repo_root)
                .map_err(|_| anyhow!("Path is outside repository: {}", path.display()))?
                .to_path_buf();
            out.insert(relative_to_root);
        } else {
            bail!("Path is not a file or directory: {}", path.display());
        }
    }

    Ok(out)
}

#[miniextendr]
pub fn dvs_add(
    paths: Vec<PathBuf>,
    message: Missing<Option<String>>,
) -> Result<DataFrame<AsSerializeRow<AddResult>>> {
    let message = if message.is_missing() {
        None
    } else {
        message.unwrap()
    };

    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;
    let all_paths: Vec<_> = resolve_paths_for_add(paths, &dvs_paths)?
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

// TODO: consider if resolve_paths_for_get can be added to dvs-lib, as it introduces `walkdir` as a dependency

fn resolve_paths_for_get(paths: Vec<PathBuf>, dvs_paths: &DvsPaths) -> Result<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    let metadata_root = dvs_paths.metadata_folder().canonicalize()?;
    // Get cwd-relative prefix for converting user paths to repo-root-relative
    let cwd_prefix = dvs_paths.cwd_relative_to_root();

    // Convert user paths to repo-relative directory filters
    // If no paths given, default to cwd (or repo root if at root)
    let dir_filters: Vec<PathBuf> = if paths.is_empty() {
        vec![cwd_prefix.map(|p| p.to_path_buf()).unwrap_or_default()]
    } else {
        paths
            .into_iter()
            .map(|p| {
                if let Some(prefix) = cwd_prefix {
                    prefix.join(&p)
                } else {
                    p
                }
            })
            .collect()
    };

    // Walk all metadata files
    for entry in walkdir::WalkDir::new(&metadata_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let entry_path = entry.path();

        // Skip directories and non .dvs files
        if !entry_path.is_file() || entry_path.extension() != Some(std::ffi::OsStr::new("dvs")) {
            continue;
        }
        // Get repo-relative tracked path (strip metadata folder and .dvs extension)
        let relative_to_metadata = match entry_path.strip_prefix(&metadata_root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let tracked_path = relative_to_metadata.with_extension("");

        // Filter: must be under one of user's directories (or exact match)
        let under_filter = dir_filters
            .iter()
            .any(|dir| tracked_path.starts_with(dir) || &tracked_path == dir);
        if !under_filter {
            continue;
        }
        out.insert(tracked_path);
    }

    Ok(out)
}

#[miniextendr]
pub fn dvs_get(paths: Vec<PathBuf>) -> Result<DataFrame<AsSerializeRow<GetResult>>> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let all_paths: Vec<_> = resolve_paths_for_get(paths, &dvs_paths)?
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
