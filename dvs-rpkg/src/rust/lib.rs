//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).
//! Results are returned as JSON strings for efficient parsing in R.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use miniextendr_api::externalptr::ExternalPtr;
use miniextendr_api::ffi::{R_NilValue, R_PreserveObject, R_ReleaseObject, SEXP};
use miniextendr_api::serde::ColumnarDataFrame;
use miniextendr_api::{AsSerializeRow, DataFrame, List, RCall, list, miniextendr, r_println};

use anyhow::{Result, anyhow};

// Re-export dvs types for internal use
use dvs::config::Config;
use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{
    AddResult, Compression, FileProgress, GetResult, Status, StatusDetail, add_files, get_files,
    get_status,
};

enum ProgressEvent<T> {
    Tick,
    Done(std::result::Result<T, String>),
}

#[derive(miniextendr_api::ExternalPtr)]
pub struct ProgressBarCallback {
    callback: SEXP,
}

impl ProgressBarCallback {
    fn call(&self) -> Result<()> {
        unsafe { RCall::from_sexp(self.callback).eval_global() }
            .map(|_| ())
            .map_err(|err| anyhow!("Progress callback failed: {err}"))
    }
}

impl Drop for ProgressBarCallback {
    fn drop(&mut self) {
        unsafe {
            if self.callback != R_NilValue {
                R_ReleaseObject(self.callback);
                self.callback = R_NilValue;
            }
        }
    }
}

#[miniextendr(internal)]
impl ProgressBarCallback {
    /// Create an internal progress-bar callback wrapper.
    pub fn new(callback: SEXP) -> Self {
        unsafe { R_PreserveObject(callback) };
        Self { callback }
    }

    /// Tick the wrapped R callback.
    pub fn tick(&self) -> Result<()> {
        self.call()
    }
}

fn run_with_progress<T, F>(progress_callback: ExternalPtr<ProgressBarCallback>, task: F) -> Result<T>
where
    T: Send,
    F: FnOnce(Sender<ProgressEvent<T>>) -> Result<T> + Send,
{
    thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        let worker = scope.spawn(move || {
            let result = task(tx.clone()).map_err(|err| err.to_string());
            let _ = tx.send(ProgressEvent::Done(result));
        });

        let mut callback_error = None;
        let mut task_result = None;

        while task_result.is_none() {
            match rx.recv() {
                Ok(ProgressEvent::Tick) => {
                    if callback_error.is_none() {
                        if let Err(err) = progress_callback.tick() {
                            callback_error = Some(err);
                        }
                    }
                }
                Ok(ProgressEvent::Done(result)) => {
                    task_result = Some(result.map_err(|err| anyhow!(err)));
                }
                Err(_) => {
                    task_result = Some(Err(anyhow!(
                        "Progress worker channel closed unexpectedly"
                    )));
                }
            }
        }

        match worker.join() {
            Ok(()) => {}
            Err(_) => return Err(anyhow!("Progress worker thread panicked")),
        }

        let result =
            task_result.expect("task result should be set when progress loop exits")?;
        if let Some(err) = callback_error {
            return Err(err);
        }
        Ok(result)
    })
}

/// Initialize a DVS repository in the given directory.
///
/// Creates the `.dvs` metadata folder and configures storage for versioned files.
///
/// @param storage_path Path to the storage directory where file contents are kept.
/// @param root_dir Repository root directory. Defaults to the current working directory.
/// @param group Unix group name to set on stored files for shared access.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @param no_compression If `TRUE`, disable compression for stored files.
#[miniextendr(r_name = "dvs_init_impl")]
pub(crate) fn dvs_init(
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
#[miniextendr(r_name = "dvs_add_impl")]
pub(crate) fn dvs_add(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] message: Option<String>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
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

    let dry_run = dry_run.unwrap_or(false);
    let results = if let Some(progress_callback) = progress_callback {
        run_with_progress(progress_callback, |progress_tx| {
            let on_file_start = move |_: &std::path::Path, _size: u64| {
                let progress_tx = progress_tx.clone();
                FileProgress {
                    on_bytes: Box::new(|_| {}),
                    on_done: Box::new(move |_| {
                        let _ = progress_tx.send(ProgressEvent::Tick);
                    }),
                }
            };
            add_files(
                all_paths.clone(),
                &paths,
                config.backend(),
                message.clone(),
                config.compression(),
                dry_run,
                Some(&on_file_start),
            )
        })?
    } else {
        add_files(
            all_paths,
            &paths,
            config.backend(),
            message,
            config.compression(),
            dry_run,
            None,
        )?
    };

    Ok(DataFrame::from_iter(
        results.into_iter().map(|x| x.into()),
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
#[miniextendr(r_name = "dvs_status_impl")]
pub(crate) fn dvs_status(
    #[miniextendr(default = "NULL")] current: Option<bool>,
    #[miniextendr(default = "NULL")] absent: Option<bool>,
    #[miniextendr(default = "NULL")] unsynced: Option<bool>,
) -> Result<ColumnarDataFrame> {
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
            StatusDetail::Success { status, .. } => {
                (current && *status == Status::Current)
                    || (absent && *status == Status::Absent)
                    || (unsynced && *status == Status::Unsynced)
            }
            StatusDetail::Error { .. } => true,
        });
    }

    Ok(miniextendr_api::serde::vec_to_dataframe(&statuses)?
        .drop("metadata_hashes_md5")
        .strip_prefix("metadata_hashes_")
        .strip_prefix("metadata_")
        .rename("blake3", "hash"))
}

/// Retrieve files from DVS storage into the working directory.
///
/// Reads `.dvs` metadata files, fetches the corresponding contents from
/// the content-addressable store, and writes them to their original paths.
///
/// @param files Character vector of `.dvs` metadata file paths to retrieve.
/// @param glob Optional glob pattern to select `.dvs` files (e.g. `"data/*.dvs"`).
/// @param dry_run If `TRUE`, report what would be retrieved without writing files.
#[miniextendr(r_name = "dvs_get_impl")]
pub(crate) fn dvs_get(
    #[miniextendr(default = "character(0)")] files: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
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

    let dry_run = dry_run.unwrap_or(false);
    let results = if let Some(progress_callback) = progress_callback {
        run_with_progress(progress_callback, |progress_tx| {
            let on_file_start = move |_: &std::path::Path, _size: u64| {
                let progress_tx = progress_tx.clone();
                FileProgress {
                    on_bytes: Box::new(|_| {}),
                    on_done: Box::new(move |_| {
                        let _ = progress_tx.send(ProgressEvent::Tick);
                    }),
                }
            };
            get_files(
                all_paths.clone(),
                &dvs_paths,
                config.backend(),
                dry_run,
                Some(&on_file_start),
            )
        })?
    } else {
        get_files(all_paths, &dvs_paths, config.backend(), dry_run, None)?
    };
    Ok(DataFrame::from_iter(results.into_iter().map(|x| x.into())))
}

miniextendr_api::miniextendr_init!(dvs);
