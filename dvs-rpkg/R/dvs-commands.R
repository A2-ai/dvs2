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
  status = c("current", "absent", "stale", "modified", "unsynced")
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
#' @param on_error What to do when get did not do as asked (a file failed, or
#'   `on_conflict = "abort"` held files back; files left by
#'   `on_conflict = "skip"` count as done): `"return"` (the default) only
#'   returns the result, `"warn"` also warns and `"error"` raises an error. The
#'   condition has class `dvs_get_incomplete` and carries the result as
#'   `cnd$result`. Pipelines can set `options(dvs.on_error = "error")`.
#' @return A tibble of class `dvs_get_result` with one row per requested file
#'   and always the same columns: `path`, `local_path`, `before` (`"absent"`,
#'   `"current"`, `"clean"`, `"modified"`, `"unknown"`, `"not_tracked"`,
#'   `"not_a_file"`, `"unsafe_path"`), `action` (`"restored"`, `"replaced"`,
#'   `"unchanged"`, `"skipped"`, `"not_run"`, `"failed"`, `"would_restore"`,
#'   `"would_replace"`), `satisfied`, `size`, `target_hash`, `local_hash`,
#'   `stored_copy`, `code`, `message`, `hint` and `duration_ms` (the
#'   milliseconds spent on the file, `NA` when it was never hashed or
#'   copied). [dvs_result_info()] returns
#'   the outcome and the policy that applied. Outside a dvs project it raises an
#'   error; configuration errors (for example `on_conflict = "overwrite"` in
#'   dvs.toml) raise a `dvs_config_error`.
#'
#'   In a dry run, `satisfied` is the prediction: `TRUE` when the file would be
#'   at its tracked version afterwards.
#'
#'   In scripts and pipelines, use `on_error = "error"` (or set
#'   `options(dvs.on_error = "error")`) so the analysis cannot continue on data
#'   that was not updated.
#' @examples
#' \dontrun{
#' # In a script: stop unless get did as asked
#' dvs_get(recursive = TRUE, on_error = "error")
#'
#' # Interactively: look at what needs attention
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
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!isTRUE(dry_run) && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  raw <- dvs_get_impl(
    paths = paths,
    glob = glob,
    recursive = recursive,
    dry_run = dry_run,
    on_conflict = on_conflict,
    on_error = on_error,
    progress_callback = progress_callback
  )
  info <- raw$info

  files <- tibble::as_tibble(raw$files)
  files$size <- new_dvs_bytes(files$size)
  result <- structure(files, class = c("dvs_get_result", class(files)), dvs_info = info)

  if (raw$on_error != "return" && !info$did_as_asked) {
    signal_incomplete(result, raw$on_error, sys.call())
  }
  result
}

dvs_get_summary <- function(info, files = NULL) {
  counts <- info$counts
  phrases <- c(
    restored = "restored %d",
    replaced = "replaced %d",
    would_restore = "would restore %d",
    would_replace = "would replace %d",
    unchanged = "%d already current",
    skipped = "skipped %d",
    not_run = "%d held back",
    cancelled = "cancelled %d",
    failed = "%d failed"
  )
  parts <- character(0)
  for (name in names(phrases)) {
    n <- counts[[name]]
    if (!is.null(n) && n > 0) {
      parts <- c(parts, sprintf(phrases[[name]], n))
    }
  }
  what <- if (!is.na(info$problem)) {
    info$problem
  } else if (info$outcome == "aborted") {
    reasons <- character(0)
    if (!is.null(files) && "code" %in% names(files)) {
      n_conflicts <- sum(files$code %in% c("local_changes", "unknown_local"))
      if (n_conflicts > 0) {
        reasons <- sprintf("%d with local changes or no record", n_conflicts)
      }
      if (counts$failed > 0) {
        reasons <- c(reasons, sprintf(
          "%d %s cannot be retrieved",
          counts$failed,
          if (counts$failed == 1) "file" else "files"
        ))
      }
    } else {
      reasons <- parts
    }
    sprintf(
      "nothing %schanged (%s)",
      if (isTRUE(info$dry_run)) "would be " else "",
      paste(reasons, collapse = ", ")
    )
  } else if (length(parts) == 0) {
    "nothing to do"
  } else {
    paste(parts, collapse = ", ")
  }
  policy <- sprintf("; on_conflict = %s (%s)", info$on_conflict, info$source)
  sprintf(
    "dvs get%s: %s, %s%s",
    if (isTRUE(info$dry_run)) " (dry run)" else "",
    info$outcome,
    what,
    policy
  )
}

# Rows that need attention, as short lines for print and conditions. Files
# merely held back by another file are summarised, not listed.
dvs_get_attention <- function(files, max = 5L) {
  todo <- files[!files$satisfied & !(files$code %in% "batch_aborted"), , drop = FALSE]
  if (nrow(todo) == 0L) {
    return(character(0))
  }
  shown <- utils::head(todo, max)
  lines <- sprintf("! %s: %s", shown$local_path, shown$message)
  hints <- ifelse(is.na(shown$hint), "", paste0("\n    ", shown$hint))
  lines <- paste0(lines, hints)
  if (nrow(todo) > max) {
    lines <- c(lines, sprintf("... and %d more: res[!res$satisfied, ]", nrow(todo) - max))
  }
  lines
}

signal_incomplete <- function(result, on_error, call) {
  message <- paste(
    c(
      dvs_get_summary(dvs_result_info(result), result),
      dvs_get_attention(result),
      "See the `code`, `message` and `hint` columns of `cnd$result`."
    ),
    collapse = "\n"
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
    n_request <- info$counts$total
    as_returned <- c("local_path", "satisfied", "code", "message", "hint")
    if (nrow(x) == n_request && all(as_returned %in% names(x))) {
      cat("# ", dvs_get_summary(info, x), "\n", sep = "")
      attention <- dvs_get_attention(x)
      if (length(attention) > 0) {
        cat(paste0("# ", gsub("\n", "\n# ", attention, fixed = TRUE)), sep = "\n")
      }
    } else {
      # Filtered or columns selected: the info describes the whole request.
      cat(
        "# ", nrow(x), " of ", n_request, " rows from: ",
        dvs_get_summary(info), "\n",
        sep = ""
      )
    }
  }
  NextMethod()
  invisible(x)
}
