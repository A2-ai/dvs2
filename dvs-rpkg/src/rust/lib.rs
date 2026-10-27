//! dvs-rpkg: Data Version Control System R Bindings
//!
//! This crate provides R bindings for the DVS (Data Version Control System).

mod cli_progress;
mod test_support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::SyncSender;

use miniextendr_api::dataframe::IntoDataFrame;
use miniextendr_api::externalptr::ExternalPtr;
use miniextendr_api::optionals::log_impl::log;
use miniextendr_api::pump::WorkerPump;
use miniextendr_api::{
    BuiltDataFrame, DataFrameRow, MatchArg, NamedListBuilder, Preserved, miniextendr,
};

use anyhow::{Result, anyhow};
use dvs::config::{Config, ConfigError, OnConflict};
use dvs::globbing::{resolve_paths_for_add, select_paths_for_get};
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{
    Compression, FileProgress, GetOptions, GetReport, GetRow, GetVerdict, OperationContext,
    PathFilter, Status, TransferObserver, add_files, get_files, set_num_threads,
};

use cli_progress::CliProgressBar;

// region: Progress channel protocol

/// Negative = file started (|value| = file size), positive = bytes transferred.
type ProgressBytes = i64;

/// Minimum bytes to accumulate before sending a channel message.
const BATCH_THRESHOLD: i64 = 256 * 1024;

/// Run `task` on a worker thread with a cli progress bar on the R main thread.
///
/// Uses `WorkerPump` to coordinate the worker and progress-bar threads.
/// `WorkerPump`'s default `drain_logs_each_tick(true)` drains the
/// cross-thread log queue on every pump tick, so worker-thread log records
/// surface in real time without a manual `drain_log_queue()` call.
fn run_with_progress<T, F>(task: F) -> Result<T>
where
    T: Send,
    F: FnOnce(SyncSender<ProgressBytes>) -> Result<T> + Send,
{
    let mut bar = CliProgressBar::new();
    let result = WorkerPump::<ProgressBytes>::new().channel_capacity(64).run(
        |tx| task(tx).map_err(Into::into),
        |bytes| {
            if bytes < 0 {
                bar.grow_total((-bytes) as f64);
            } else {
                bar.add(bytes as f64);
            }
        },
    );
    bar.done();
    result.map_err(|e| anyhow!("{e}"))
}

/// Build the `on_file_start` closure that feeds byte-level progress.
fn progress_on_file_start(
    tx: SyncSender<ProgressBytes>,
) -> impl Fn(&std::path::Path, u64) -> FileProgress + Send + Sync {
    move |_path: &std::path::Path, size: u64| {
        let size_i64 = i64::try_from(size).expect("file size exceeds i64::MAX");
        let _ = tx.send(-size_i64);

        let pending = Arc::new(AtomicI64::new(0));
        let tx_bytes = tx.clone();
        let tx_done = tx.clone();
        let pending_bytes = Arc::clone(&pending);
        let pending_done = Arc::clone(&pending);

        FileProgress {
            on_bytes: Box::new(move |n| {
                let acc = pending_bytes.fetch_add(n as i64, Ordering::Relaxed) + n as i64;
                if acc >= BATCH_THRESHOLD {
                    // Benign race: concurrent callers may both exceed the threshold and
                    // swap; one gets all accumulated bytes, the other gets 0 and no-ops.
                    // No bytes are lost — just batched unevenly. Fine for UI.
                    let flushed = pending_bytes.swap(0, Ordering::Relaxed);
                    if flushed > 0 {
                        let _ = tx_bytes.send(flushed);
                    }
                }
            }),
            on_done: Box::new(move |_| {
                let remaining = pending_done.swap(0, Ordering::Relaxed);
                if remaining > 0 {
                    let _ = tx_done.send(remaining);
                }
            }),
        }
    }
}

// endregion: Progress channel protocol

// region: ProgressBarCallback

#[derive(Default, miniextendr_api::ExternalPtr)]
pub struct ProgressBarCallback;

#[miniextendr(internal)]
impl ProgressBarCallback {
    /// Create a progress callback handle (signals that progress should be shown).
    pub fn new() -> Self {
        Self
    }
}

// endregion: ProgressBarCallback

// region: DVS operations

/// Valid compression choices for [`dvs_init`].
#[derive(Copy, Clone, Debug, PartialEq, miniextendr_api::MatchArg)]
#[match_arg(rename_all = "lower")]
pub enum CompressionChoice {
    /// Compress stored files with zstd (default).
    Zstd,
    /// Store files without compression.
    None,
}

impl From<CompressionChoice> for Compression {
    fn from(c: CompressionChoice) -> Self {
        match c {
            CompressionChoice::Zstd => Compression::Zstd,
            CompressionChoice::None => Compression::None,
        }
    }
}

/// @title Initialize a DVS repository in the given directory
/// @description
/// Creates the `.dvs` metadata folder and configures storage for versioned files.
///
/// @param storage_path Path to the storage directory where file contents are kept.
/// @param root_dir Repository root directory. Defaults to the current working directory.
/// @param group Unix group name to set on stored files for shared access.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @param compression Compression method for stored files. One of `"zstd"`
///   (default) or `"none"`.
/// @return A single-row data frame describing the resulting configuration:
///   `compression`, `metadata_folder_name`, `backend_path`, `backend_group`.
///   An optional config field that was not set comes back as `NA`.
/// @keywords internal
#[miniextendr(r_name = "dvs_init_impl", internal, call = caller)]
pub(crate) fn dvs_init(
    storage_path: PathBuf,
    #[miniextendr(default = "NULL")] root_dir: Option<PathBuf>,
    #[miniextendr(default = "NULL")] group: Option<String>,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
    #[miniextendr(match_arg, default = "\"zstd\"")] compression: CompressionChoice,
) -> Result<BuiltDataFrame> {
    let root_dir = match root_dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let mut config = Config::new_local(&storage_path, group)?;

    config.set_compression(Some(compression.into()));
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }

    // Re-read the config from disk after init so the returned frame reflects
    // exactly what was persisted to dvs.toml — notably the absolute
    // `backend_path` that dvs core canonicalizes — rather than the pre-init
    // input. Keeps this rpkg-only: core canonicalizes, we just read the result.
    let repo_root = init(&root_dir, config)?;
    let config = Config::find(&repo_root)
        .ok_or_else(|| anyhow!("dvs.toml not found after init at {}", repo_root.display()))??;

    // `metadata_folder_name` is an `Option<String>`: when set it comes back as a
    // plain character scalar, when unset (None) as a scalar `NA` (logical)
    // column. Neither is a list-column, so init rows stack with
    // rbind()/bind_rows(). Relies on miniextendr#307 (fixed); the R tests in
    // test-init.R guard both the set and unset cases.
    Ok(miniextendr_api::serde::vec_to_dataframe(&[config])?)
}

/// @title Add files to DVS-managed storage
/// @description
/// Hashes and copies the specified files into DVS storage.
///
/// @param paths Character vector of file paths to add to DVS storage.
/// @param message Optional commit message describing why the files were added.
/// @param glob Optional glob pattern to select files (e.g. `"data/*.csv"`).
///   Globs use a literal path separator: `*.csv` only matches files in the
///   target directory and will not match `subdir/file.csv`. Use `**/*.csv` to
///   match recursively across subdirectories.
/// @param dry_run If `TRUE`, report what would be added without modifying anything.
/// @param progress_callback Optional handle to enable progress bar display.
/// @keywords internal
#[miniextendr(r_name = "dvs_add_impl", internal, call = caller)]
pub(crate) fn dvs_add(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] message: Option<String>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
) -> Result<BuiltDataFrame> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
        .into_iter()
        .collect();
    if all_paths.is_empty() {
        return Err(anyhow!("No files to add"));
    }

    let dry_run = dry_run.unwrap_or(false);
    let results = if progress_callback.is_some() {
        run_with_progress(|tx| {
            let ctx = OperationContext::new()
                .with_observer(Arc::new(TransferObserver(progress_on_file_start(tx))));
            add_files(
                all_paths.clone(),
                &dvs_paths,
                config.backend(),
                message.clone(),
                config.compression(),
                dry_run,
                &ctx,
            )
            .map(|report| report.files)
        })?
    } else {
        add_files(
            all_paths,
            &dvs_paths,
            config.backend(),
            message,
            config.compression(),
            dry_run,
            &OperationContext::new(),
        )?
        .files
    };

    Ok(miniextendr_api::serde::vec_to_dataframe(&results)?)
}

/// Valid status filter choices for [`dvs_status`].
///
/// Used with `match.arg(status, several.ok = TRUE)` in the R wrapper to let
/// users select which file statuses to include.
#[derive(Copy, Clone, Debug, PartialEq, miniextendr_api::MatchArg)]
#[match_arg(rename_all = "lower")]
pub enum StatusChoice {
    /// Local file exists and matches stored version.
    Current,
    /// Metadata exists but local file is missing.
    Absent,
    /// Local file matches the last successful get, while metadata has advanced.
    Stale,
    /// Local file differs from metadata and the last successful get.
    Modified,
    /// Local file differs from metadata, but no local baseline is available.
    Unsynced,
}

impl From<StatusChoice> for Status {
    fn from(c: StatusChoice) -> Self {
        match c {
            StatusChoice::Current => Status::Current,
            StatusChoice::Absent => Status::Absent,
            StatusChoice::Stale => Status::Stale,
            StatusChoice::Modified => Status::Modified,
            StatusChoice::Unsynced => Status::Unsynced,
        }
    }
}

/// @title Report the sync status of DVS-managed files
/// @description
/// Reports the sync status of DVS-managed files. By default all statuses
/// are shown; pass a character vector
/// of status names (e.g. `c("current", "absent")`) to restrict output.
/// A file whose status cannot be read is always kept, with status `"error"`
/// and the reason in `error`.
///
/// @param paths Character vector of file or directory paths to check status for.
/// @param recursive If `TRUE`, directory inputs include all descendants;
///   if `FALSE` or `NULL` (default), only direct children of the directory
///   are returned. The flag only constrains descendants of paths passed
///   explicitly — when `paths` is empty, every tracked file is returned
///   regardless of nesting depth, and `recursive` has no effect.
/// @param glob Optional glob pattern to select files (e.g. `"data/*.csv"`).
///   Globs use a literal path separator: `*.csv` only matches files in the
///   target directory and will not match `subdir/file.csv`. Use `**/*.csv` to
///   match recursively across subdirectories. Mutually exclusive with `recursive`.
/// @param status Character vector of statuses to include. Valid values are
///   `"current"`, `"absent"`, `"stale"`, `"modified"`, and `"unsynced"`. When empty (default), all
///   statuses are shown.
/// @keywords internal
#[miniextendr(r_name = "dvs_status_impl", internal, call = caller)]
pub(crate) fn dvs_status(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] recursive: Option<bool>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(match_arg, several_ok)] status: Vec<StatusChoice>,
) -> Result<BuiltDataFrame> {
    let current_dir = std::env::current_dir()?;

    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    if glob.is_some() && recursive.unwrap_or(false) {
        return Err(anyhow!("`glob` and `recursive` are mutually exclusive"));
    }

    let show_all = status.is_empty() || status.len() == StatusChoice::CHOICES.len();

    let filter = if paths.is_empty() {
        None
    } else {
        Some(PathFilter::from_user_paths(
            paths,
            recursive.unwrap_or(false),
            &dvs_paths,
        ))
    };
    let mut statuses = dvs::inspect_status(
        &dvs_paths,
        filter.as_ref(),
        glob.as_deref(),
        &OperationContext::new(),
    )?
    .files;
    if !show_all {
        statuses.retain(|row| {
            row.error.is_some() || status.iter().any(|c| row.status == Some(Status::from(*c)))
        });
    }

    // DataFrameRow has no datetime columns, so add_time joins as a POSIXct
    // vector converted from the jiff timestamps. Rooted until the frame holds
    // it: with_column allocates the new frame before it stores the column.
    let add_time = Preserved::new(
        statuses
            .iter()
            .map(|s| s.metadata.as_ref().map(|m| m.add_time))
            .collect::<Vec<Option<_>>>(),
    );
    let rows: Vec<StatusRow> = statuses.iter().map(StatusRow::from).collect();
    Ok(rows
        .into_dataframe()?
        .with_column("add_time", add_time.as_sexp()))
}

/// @title Materialize the audit log as a data frame
/// @description
/// Reads the audit log of the DVS repository containing the working
/// directory and returns one row per entry: `operation_id`, `timestamp`
/// (POSIXct, UTC) and `user`, then the entry's action flattened into
/// columns. `action_variant` is `"init"` or `"add"`. The other `action_`
/// columns hold that action's fields, for an add `action_file_path`,
/// `action_file_hashes_blake3`, `action_compression` and `action_message`,
/// and are `NA` where the action has no such field. The columns follow
/// `dvs::audit::AuditEntry`'s serde representation, so the rpkg never
/// reaches into the dvs crate's internal types.
///
/// @param paths Character vector of repo-root-relative file paths recorded at
///   add time. The filter keeps only `add` entries for those files. An empty
///   vector (the default) returns the whole log, including `init` entries. A
///   non-empty filter drops all `init` entries.
/// @keywords internal
#[miniextendr(r_name = "dvs_audit_log_impl", internal, call = caller)]
pub(crate) fn dvs_audit_log(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
) -> Result<BuiltDataFrame> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;

    // An empty `paths` slice returns the whole audit log. A non-empty filter
    // keeps only `add` entries recording one of those repo-root-relative paths
    // and drops `init` entries. Entries are deserialized off the JSONL via the
    // dvs crate's own serde impls and handed straight to the dataframe
    // serializer, so the structure is inherited through serde. vec_to_dataframe
    // returns an empty data frame when the filter matches nothing.
    let entries = config.backend().get_audit_entries(&paths)?;
    let frame = miniextendr_api::serde::vec_to_dataframe_flatten_enums(&entries, &["action"])?;
    if entries.is_empty() {
        return Ok(frame);
    }
    // The log keeps unix seconds. Report them as POSIXct, as the CLI reports
    // RFC 3339 times. Rooted until the frame holds the column.
    let timestamp = Preserved::new(entries.iter().map(|e| e.timestamp).collect::<Vec<_>>());
    Ok(frame.with_column("timestamp", timestamp.as_sexp()))
}

// region: status rows

/// One row of the `dvs_status()` frame, without `add_time`. The columns are
/// fixed: a row without metadata, or without a message or error, fills them
/// with `NA`.
#[derive(DataFrameRow)]
struct StatusRow {
    path: String,
    status: String,
    hash: Option<String>,
    size: Option<f64>,
    created_by: Option<String>,
    compression: Option<String>,
    message: Option<String>,
    error: Option<String>,
}

impl From<&dvs::StatusFacts> for StatusRow {
    fn from(row: &dvs::StatusFacts) -> Self {
        let metadata = row.metadata.as_ref();
        StatusRow {
            path: row.path.to_string_lossy().into_owned(),
            status: row.status_name(),
            hash: metadata.map(|m| m.hashes.blake3.clone()),
            size: metadata.map(|m| m.size as f64),
            created_by: metadata.map(|m| m.created_by.clone()),
            compression: metadata.map(|m| m.compression.to_string()),
            message: metadata.and_then(|m| m.message.clone()),
            error: row.error.clone(),
        }
    }
}

// endregion

/// Valid `on_conflict` choices for [`dvs_get`].
#[derive(Copy, Clone, Debug, PartialEq, miniextendr_api::MatchArg)]
#[match_arg(rename_all = "lower")]
pub enum OnConflictChoice {
    /// Change no files if any requested file has a conflict.
    Abort,
    /// Leave conflicting files as they are and get the rest.
    Skip,
    /// Replace conflicting files.
    Overwrite,
}

impl From<OnConflictChoice> for OnConflict {
    fn from(c: OnConflictChoice) -> Self {
        match c {
            OnConflictChoice::Abort => OnConflict::Abort,
            OnConflictChoice::Skip => OnConflict::Skip,
            OnConflictChoice::Overwrite => OnConflict::Overwrite,
        }
    }
}

/// @title Retrieve files from DVS storage into the working directory
/// @description
/// Fetches the specified files from DVS storage and writes them
/// to their original paths in the working directory. With no `paths`
/// or `glob`, `dvs_get()` scopes to the working directory: it restores
/// the files directly under it, and `dvs_get(recursive = TRUE)` restores
/// every tracked file beneath it at any depth.
///
/// A working file that differs from the tracked version is replaced without
/// asking only when dvs itself put it there (for example after switching git
/// branches). Otherwise it is a conflict, handled by `on_conflict`.
///
/// @param paths Character vector of file paths to retrieve from DVS storage.
/// @param glob Optional glob pattern to select files (e.g. `"data/*.csv"`).
///   Globs use a literal path separator: `*.csv` only matches files in the
///   target directory and will not match `subdir/file.csv`. Use `**/*.csv` to
///   match recursively across subdirectories. Mutually exclusive with `recursive`.
/// @param recursive If `TRUE`, directory inputs include all descendants;
///   if `FALSE` or `NULL` (default), only direct children of the directory
///   are returned. With no explicit `paths` the directory is the working
///   directory. Mutually exclusive with `glob`.
/// @param dry_run If `TRUE`, report what would be retrieved without writing files.
/// @param on_conflict `NULL` (use `DVS_GET_ON_CONFLICT`, then `[get] on_conflict`
///   in dvs.toml, then `"abort"`), or one of `"abort"` (change no files if any
///   requested file has local changes or another problem), `"skip"` (leave those
///   files and get the rest) or `"overwrite"` (replace them; local changes are lost).
/// @param progress_callback Optional handle to enable progress bar display.
/// @keywords internal
#[miniextendr(r_name = "dvs_get_impl", internal, call = caller)]
pub(crate) fn dvs_get(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] recursive: Option<bool>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(match_arg)] on_conflict: Option<OnConflictChoice>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
) -> Result<NamedListBuilder> {
    let dry_run = dry_run.unwrap_or(false);
    let explicit = on_conflict.map(OnConflict::from);
    if glob.is_some() && recursive.unwrap_or(false) {
        return Err(anyhow!("`glob` and `recursive` are mutually exclusive"));
    }
    let current_dir = std::env::current_dir()?;
    let config =
        match Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))? {
            Err(e) => config_error(&e.to_string()),
            Ok(c) => c,
        };
    let (policy, source) = match config.get_on_conflict(explicit) {
        Ok(p) => p,
        Err(ConfigError(message)) => config_error(&message),
    };
    let dvs_paths = DvsPaths::from_cwd(&config)?;
    let selection = select_paths_for_get(
        paths,
        glob.as_deref(),
        &dvs_paths,
        recursive.unwrap_or(false),
    )?;
    let options = GetOptions {
        dry_run,
        on_conflict: policy,
        source,
    };

    let report = if progress_callback.is_some() && !dry_run {
        run_with_progress(|tx| {
            let ctx = OperationContext::new()
                .with_observer(Arc::new(TransferObserver(progress_on_file_start(tx))));
            Ok(get_files(
                selection,
                &dvs_paths,
                config.backend(),
                options,
                &ctx,
            ))
        })?
    } else {
        get_files(
            selection,
            &dvs_paths,
            config.backend(),
            options,
            &OperationContext::new(),
        )
    };
    get_result_list(&report)
}

/// Raises a `dvs_config_error`, one of the `dvs_error` family.
fn config_error(message: &str) -> ! {
    miniextendr_api::error!(class = ["dvs_config_error", "dvs_error"], "{message}")
}

/// One row of the `files` frame of a `dvs_get()` result.
#[derive(DataFrameRow)]
struct GetFileRow {
    path: String,
    local_path: String,
    before: String,
    action: String,
    satisfied: bool,
    size: Option<f64>,
    target_hash: Option<String>,
    local_hash: Option<String>,
    stored_copy: Option<String>,
    code: Option<String>,
    message: Option<String>,
    hint: Option<String>,
}

impl From<&GetRow> for GetFileRow {
    fn from(r: &GetRow) -> Self {
        GetFileRow {
            path: r.path.to_string_lossy().into_owned(),
            local_path: r.local_path.to_string_lossy().into_owned(),
            before: r.before.as_str().to_string(),
            action: r.action.as_str().to_string(),
            satisfied: r.satisfied,
            size: r.size.map(|s| s as f64),
            target_hash: r.target_hash.clone(),
            local_hash: r.local_hash.clone(),
            stored_copy: r.stored_copy.map(|c| c.as_str().to_string()),
            code: r.code.map(|c| c.as_str().to_string()),
            message: r.message.clone(),
            hint: r.hint.clone(),
        }
    }
}

/// `list(files = <data.frame with a fixed schema>, info = <list>)`.
fn get_result_list(report: &GetReport) -> Result<NamedListBuilder> {
    let rows: Vec<GetFileRow> = report.files.iter().map(GetFileRow::from).collect();

    let c = &report.counts;
    let mut counts = NamedListBuilder::with_capacity(8);
    for (name, n) in [
        ("restored", c.restored),
        ("replaced", c.replaced),
        ("unchanged", c.unchanged),
        ("skipped", c.skipped),
        ("not_run", c.not_run),
        ("failed", c.failed),
        ("would_restore", c.would_restore),
        ("would_replace", c.would_replace),
    ] {
        counts.push(name, n as i32);
    }

    let mut info = NamedListBuilder::with_capacity(8);
    info.push("command", "get");
    info.push("outcome", report.outcome.as_str());
    info.push("did_as_asked", report.verdict() == GetVerdict::Done);
    info.push("dry_run", report.dry_run);
    info.push("on_conflict", report.on_conflict.as_str());
    info.push("source", report.source.as_str());
    info.push("counts", counts);
    info.push(
        "problem",
        report.problem.as_ref().map(|p| p.message.clone()),
    );

    let mut result = NamedListBuilder::with_capacity(2);
    result.push("files", rows.into_dataframe()?);
    result.push("info", info);
    Ok(result)
}

/// @title Set the number of threads used by DVS parallel operations
/// @description
/// Controls the thread pool size for add, get, and status operations.
/// Pass `NULL` to revert to automatic detection.
///
/// @param threads Integer number of threads, or `NULL` to reset.
/// @keywords internal
#[miniextendr(r_name = "dvs_set_threads_impl", internal, call = caller)]
pub(crate) fn dvs_set_threads(#[miniextendr(default = "NULL")] threads: Option<usize>) {
    set_num_threads(threads.unwrap_or(0));
}

/// @title Format a byte count as a human-readable size string
/// @param size_bytes non-negative integer representing file sizes in bytes.
#[miniextendr]
pub fn format_byte_size(size_bytes: u64) -> String {
    dvs::format_size(size_bytes)
}

/// @title Version of the bundled DVS Rust core crate
/// @return The version of the `dvs` Rust crate compiled into this package, as a
///   string (e.g. `"0.3.0"`).
#[miniextendr]
pub fn dvs_version() -> String {
    dvs::VERSION.to_string()
}

/// @title Set the log level for DVS internals
/// @description
/// Controls which log messages from the DVS internals are routed to R's
/// console. `error` and `warn` go to [stderr()]; `info`, `debug`, and `trace`
/// go to stdout.
///
/// The R package does not consult the `RUST_LOG` environment variable.
/// `RUST_LOG=dvs=<level>` only affects the `dvs` CLI binary; in R you must
/// call `set_dvs_log_level()`.
///
/// Default at package load: `"off"` — no log output is produced until this
/// function is called.
///
/// @param level Character string giving the desired log level. The default
///   is `"off"`.
///
/// @return Called for its side effect; returns `NULL` invisibly.
///
/// @examples
/// \dontrun{
/// set_dvs_log_level("info")    # opt in
/// set_dvs_log_level("debug")
/// set_dvs_log_level("off")     # restore default (silent)
/// }
#[miniextendr(invisible)]
pub fn set_dvs_log_level(#[miniextendr(match_arg)] level: log::LevelFilter) {
    log::set_max_level(level);
}

miniextendr_api::miniextendr_init!(dvs);
