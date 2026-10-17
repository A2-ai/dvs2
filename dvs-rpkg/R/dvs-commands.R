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
  compression <- match.arg(compression)
  result <- dvs_init_impl(
    storage_path = storage_path,
    root_dir = root_dir,
    group = group,
    metadata_folder_name = metadata_folder_name,
    compression = compression
  )
  tibble::as_tibble(result)
}

#' @inherit dvs_add_impl title description params
#' @rdname dvs_add
#' @export
dvs_add <- function(
  paths = character(0),
  message = NULL,
  glob = NULL,
  dry_run = NULL
) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!isTRUE(dry_run) && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  result <- dvs_add_impl(
    paths = paths,
    message = message,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  )

  if (!is.null(result$size)) {
    result$size <- new_dvs_bytes(result$size)
  }
  if (!is.null(result$stored_size)) {
    result$stored_size <- new_dvs_bytes(result$stored_size)
  }

  tibble::as_tibble(result)
}

#' @inherit dvs_status_impl title description params
#' @rdname dvs_status
#'
#'
#' @export
dvs_status <- function(
  paths = character(0),
  recursive = NULL,
  glob = NULL,
  status = c("current", "absent", "unsynced")
) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  status_data_frame <-
    dvs_status_impl(
      paths = paths,
      recursive = recursive,
      glob = glob,
      status = status
    )

  if (!is.null(status_data_frame$size)) {
    status_data_frame$size <- new_dvs_bytes(status_data_frame$size)
  }

  tibble::as_tibble(status_data_frame)
}

#' @inherit dvs_audit_log_impl title description params
#' @rdname dvs_audit_log
#' @keywords internal
dvs_audit_log <- function(paths = character(0)) {
  tibble::as_tibble(dvs_audit_log_impl(paths = paths))
}

#' @inherit dvs_get_impl title description params
#' @param recursive If `TRUE`, directory inputs include all descendants;
#'   otherwise only direct children of the directory. With no explicit `paths`
#'   the directory is the working directory. Mutually exclusive with `glob`.
#' @param dry_run If `TRUE`, report what would happen without writing files.
#' @param on_error What to do when the request is not complete (some file is
#'   not at its tracked version afterwards): `"return"` (the default) only
#'   returns the result, `"warn"` also warns and `"error"` raises an error. The
#'   condition has class `dvs_get_incomplete` and carries the result as
#'   `cnd$result`. Pipelines can set `options(dvs.on_error = "error")`.
#' @return A tibble of class `dvs_get_result` with one row per requested file
#'   and always the same columns: `path`, `local_path`, `before` (`"absent"`,
#'   `"current"`, `"clean"`, `"modified"`, `"unknown"`, `"not_tracked"`,
#'   `"not_a_file"`, `"unsafe_path"`), `action` (`"restored"`, `"replaced"`,
#'   `"unchanged"`, `"skipped"`, `"not_run"`, `"failed"`, `"would_restore"`,
#'   `"would_replace"`), `satisfied`, `size`, `target_hash`, `local_hash`,
#'   `stored_copy`, `code`, `message` and `hint`. [dvs_result_info()] returns
#'   the outcome and the policy that applied. Configuration errors (for example
#'   `on_conflict = "overwrite"` in dvs.toml) raise a `dvs_config_error`.
#' @examples
#' \dontrun{
#' res <- dvs_get(recursive = TRUE)
#' res[!res$satisfied, c("path", "code", "hint")]
#' dvs_get("data/derived.csv", on_conflict = "overwrite")
#' }
#' @rdname dvs_get
#' @export
dvs_get <- function(
  paths = character(0),
  glob = NULL,
  recursive = FALSE,
  dry_run = FALSE,
  on_conflict = NULL,
  on_error = getOption("dvs.on_error", "return")
) {
  if (!is.character(paths) || anyNA(paths)) {
    stop("`paths` must be a character vector without NA", call. = FALSE)
  }
  if (!isTRUE(recursive) && !isFALSE(recursive)) {
    stop("`recursive` must be TRUE or FALSE", call. = FALSE)
  }
  if (!isTRUE(dry_run) && !isFALSE(dry_run)) {
    stop("`dry_run` must be TRUE or FALSE", call. = FALSE)
  }
  if (!is.null(on_conflict)) {
    on_conflict <- match.arg(on_conflict, c("abort", "skip", "overwrite"))
  }
  on_error <- match.arg(on_error, c("return", "warn", "error"))

  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!dry_run && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  raw <- dvs_get_impl(
    paths = paths,
    glob = glob,
    recursive = recursive,
    dry_run = dry_run,
    on_conflict = on_conflict,
    progress_callback = progress_callback
  )
  info <- raw$info
  if (isTRUE(info$config_error)) {
    stop(structure(
      class = c("dvs_config_error", "dvs_error", "error", "condition"),
      list(message = info$problem, call = sys.call())
    ))
  }
  info$config_error <- NULL

  files <- tibble::as_tibble(raw$files)
  files$size <- new_dvs_bytes(files$size)
  result <- structure(files, class = c("dvs_get_result", class(files)), dvs_info = info)

  if (on_error != "return" && info$outcome != "complete") {
    signal_incomplete(result, on_error, sys.call())
  }
  result
}

#' Operation information for a dvs result
#'
#' The rows of a [dvs_get()] result describe each file. This returns what
#' applies to the whole operation.
#'
#' @param x A result returned by [dvs_get()].
#' @return A list: `command`, `outcome` (`"complete"`, `"aborted"`,
#'   `"partial"` or `"failed"`), `dry_run`, `on_conflict` and its `source`
#'   (`"argument"`, `"env"`, `"config"` for dvs.toml, or `"default"`), `counts` (a named
#'   list of row counts per action) and `problem` (an operation-level message,
#'   or `NULL`).
#' @export
dvs_result_info <- function(x) {
  info <- attr(x, "dvs_info", exact = TRUE)
  if (is.null(info)) {
    stop("`x` is not a dvs result", call. = FALSE)
  }
  info
}

dvs_get_summary <- function(info) {
  counts <- info$counts
  phrases <- c(
    restored = "restored %d",
    replaced = "replaced %d",
    would_restore = "would restore %d",
    would_replace = "would replace %d",
    unchanged = "%d already current",
    skipped = "skipped %d with local changes",
    not_run = "%d held back",
    failed = "%d failed"
  )
  parts <- character(0)
  for (name in names(phrases)) {
    n <- counts[[name]]
    if (!is.null(n) && n > 0) {
      parts <- c(parts, sprintf(phrases[[name]], n))
    }
  }
  what <- if (!is.null(info$problem)) {
    info$problem
  } else if (info$outcome == "aborted") {
    sprintf(
      "nothing %schanged (%s)",
      if (isTRUE(info$dry_run)) "would be " else "",
      paste(parts, collapse = ", ")
    )
  } else if (length(parts) == 0) {
    "nothing to do"
  } else {
    paste(parts, collapse = ", ")
  }
  sprintf(
    "dvs get%s: %s, %s; on_conflict = %s (%s)",
    if (isTRUE(info$dry_run)) " (dry run)" else "",
    info$outcome,
    what,
    info$on_conflict,
    info$source
  )
}

signal_incomplete <- function(result, on_error, call) {
  message <- paste0(
    dvs_get_summary(dvs_result_info(result)),
    "\nSee the `code`, `message` and `hint` columns of the result."
  )
  type <- if (on_error == "error") c("dvs_error", "error") else c("dvs_warning", "warning")
  cnd <- structure(
    class = c("dvs_get_incomplete", type, "condition"),
    list(message = message, call = call, result = result)
  )
  if (on_error == "error") stop(cnd) else warning(cnd)
}

#' @export
print.dvs_get_result <- function(x, ...) {
  info <- attr(x, "dvs_info", exact = TRUE)
  if (!is.null(info)) {
    cat("# ", dvs_get_summary(info), "\n", sep = "")
  }
  NextMethod()
  invisible(x)
}
