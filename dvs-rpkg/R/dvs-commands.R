#' @inherit dvs_init_impl title description
#' @inherit dvs_init_impl params
#' @rdname dvs_init
#' @export
dvs_init <- function(
  storage_path,
  root_dir = NULL,
  group = NULL,
  metadata_folder_name = NULL,
  compression = c("zstd", "none")
) {
  check_string(storage_path, "storage_path", null_ok = FALSE)
  check_string(root_dir, "root_dir")
  check_string(group, "group")
  check_string(metadata_folder_name, "metadata_folder_name")
  compression <- check_choice(compression, "compression", c("zstd", "none"))
  result <- dvs_call(dvs_init_impl(
    storage_path = storage_path,
    root_dir = root_dir,
    group = group,
    metadata_folder_name = metadata_folder_name,
    compression = compression
  ))
  tibble::as_tibble(result)
}

#' @inherit dvs_add_impl title description params
#' @param on_error What to do when some files fail: `"stop"` (the default)
#'   raises a `dvs_error_files_failed` error whose `results` field holds every
#'   file's result; `"collect"` returns the results, with the failures' `error`
#'   and `code`.
#' @return A tibble, one row per file: `path`, `outcome`, `hash`, `size`,
#'   `stored_size`, and for failed files `error` and `code`.
#' @seealso [dvs_error] for the error's fields.
#' @rdname dvs_add
#' @export
dvs_add <- function(
  paths = character(0),
  message = NULL,
  glob = NULL,
  dry_run = NULL,
  on_error = c("stop", "collect")
) {
  check_strings(paths, "paths")
  check_string(message, "message")
  check_string(glob, "glob")
  check_flag(dry_run, "dry_run")
  on_error <- check_choice(on_error, "on_error", c("stop", "collect"))
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!isTRUE(dry_run) && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  result <- dvs_call(dvs_add_impl(
    paths = paths,
    message = message,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  ))

  if (!is.null(result$size)) {
    result$size <- new_dvs_bytes(result$size)
  }
  if (!is.null(result$stored_size)) {
    result$stored_size <- new_dvs_bytes(result$stored_size)
  }

  dvs_files_failed(tibble::as_tibble(result), "add", on_error)
}

#' @inherit dvs_status_impl title description params
#' @param remote If `TRUE`, also compare each file with the server project:
#'   adds the columns `remote_status` (`"latest"`: your content is the newest
#'   uploaded at its path; `"newer_upload_exists"`: something was uploaded
#'   there since, maybe from a branch that is never merged;
#'   `"unknown_path"`: nothing was ever uploaded there;
#'   `"not_placed_here"`: other content was), `on_server` (the server has
#'   this content), `yanked`, `yank_reason`, `newer_uploads`, and the newest
#'   upload's `latest_hash`, `latest_at`, `latest_by` and `latest_branch`.
#'   Server projects only.
#' @rdname dvs_status
#'
#'
#' @export
dvs_status <- function(
  paths = character(0),
  recursive = NULL,
  glob = NULL,
  status = c("current", "absent", "unsynced"),
  remote = FALSE
) {
  check_strings(paths, "paths")
  check_flag(recursive, "recursive")
  check_string(glob, "glob")
  status <- check_choices(status, "status", c("current", "absent", "unsynced"))
  check_flag(remote, "remote", null_ok = FALSE)
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  status_data_frame <-
    dvs_call(dvs_status_impl(
      paths = paths,
      recursive = recursive,
      glob = glob,
      status = status
    ))

  if (!is.null(status_data_frame$size)) {
    status_data_frame$size <- new_dvs_bytes(status_data_frame$size)
  }

  result <- tibble::as_tibble(status_data_frame)
  if (isTRUE(remote)) {
    result <- dvs_with_remote_status(result)
  }
  result
}

# The status rows with how each file compares with the server.
dvs_with_remote_status <- function(result, call = sys.call(-1)) {
  force(call)
  tracked <- if ("hash" %in% names(result)) !is.na(result[["hash"]]) else logical(nrow(result))
  remote <- if (any(tracked)) {
    dvs_call(
      dvs_remote_status_impl(
        paths = as.character(result[["path"]][tracked]),
        hashes = as.character(result[["hash"]][tracked])
      ),
      call = call
    )
  } else {
    # Still says when this is not a server project
    dvs_call(dvs_remote_status_impl(paths = character(0), hashes = character(0)), call = call)
  }
  remote <- dvs_frame(remote, dvs_spec$remote_status)
  at <- match(as.character(result[["path"]]), remote$path)
  for (column in setdiff(names(remote), "path")) {
    result[[column]] <- remote[[column]][at]
  }
  result
}

#' @inherit dvs_audit_log_impl title description params
#' @rdname dvs_audit_log
#' @keywords internal
dvs_audit_log <- function(paths = character(0)) {
  check_strings(paths, "paths")
  entries <- dvs_call(dvs_audit_log_impl(paths = paths))
  tibble::as_tibble(entries)
}

#' @inherit dvs_get_impl title description params
#' @param on_error What to do when some files fail: `"stop"` (the default)
#'   raises a `dvs_error_files_failed` error whose `results` field holds every
#'   file's result; `"collect"` returns the results, with the failures' `error`
#'   and `code`.
#' @details `dvs_get()` makes each file match its `.dvs` pointer: a file you
#'   changed locally is overwritten with the version the pointer names.
#' @return A tibble, one row per file: `path`, `outcome`, `size`, and for
#'   failed files `error` and `code`.
#' @seealso [dvs_error] for the error's fields.
#' @rdname dvs_get
#' @export
dvs_get <- function(
  paths = character(0),
  glob = NULL,
  recursive = NULL,
  dry_run = NULL,
  on_error = c("stop", "collect")
) {
  check_strings(paths, "paths")
  check_string(glob, "glob")
  check_flag(recursive, "recursive")
  check_flag(dry_run, "dry_run")
  on_error <- check_choice(on_error, "on_error", c("stop", "collect"))
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!isTRUE(dry_run) && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  get_data_frame <- dvs_call(dvs_get_impl(
    paths = paths,
    glob = glob,
    recursive = recursive,
    dry_run = dry_run,
    progress_callback = progress_callback
  ))

  if (!is.null(get_data_frame$size)) {
    get_data_frame$size <- new_dvs_bytes(get_data_frame$size)
  }

  dvs_files_failed(tibble::as_tibble(get_data_frame), "get", on_error)
}

# Stop when some files failed, unless the caller collects the failures.
dvs_files_failed <- function(results, action, on_error, call = sys.call(-1)) {
  force(call)
  errors <- if ("error" %in% names(results)) results[["error"]] else rep(NA_character_, nrow(results))
  failed <- !is.na(errors)
  if ("outcome" %in% names(results)) {
    failed <- failed | results[["outcome"]] %in% c("failed", "not_attempted")
  }
  if (on_error == "collect" || !any(failed)) {
    return(results)
  }
  shown <- utils::head(which(failed), 5L)
  lines <- sprintf(
    "%s: %s",
    results[["path"]][shown],
    ifelse(is.na(errors[shown]), "not attempted", errors[shown])
  )
  more <- sum(failed) - length(shown)
  stop(new_dvs_error(
    "files_failed",
    paste(
      c(
        sprintf("%d of %d files failed to %s:", sum(failed), nrow(results), action),
        paste0("* ", lines),
        if (more > 0L) sprintf("... and %d more", more),
        "The error's `results` field has every file's result; use on_error = \"collect\" to get them back instead."
      ),
      collapse = "\n"
    ),
    results = results,
    call = call
  ))
}
