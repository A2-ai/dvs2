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
#' @rdname dvs_get
#' @export
dvs_get <- function(paths = character(0), glob = NULL, recursive = NULL, dry_run = NULL) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  # Only show a progress bar in interactive sessions; non-interactive callers
  # should prefer dvs-cli (see #118).
  if (!isTRUE(dry_run) && interactive()) {
    progress_callback <- ProgressBarCallback$new()
  }

  get_data_frame <- dvs_get_impl(
    paths = paths,
    glob = glob,
    recursive = recursive,
    dry_run = dry_run,
    progress_callback = progress_callback
  )

  if (!is.null(get_data_frame$size)) {
    get_data_frame$size <- new_dvs_bytes(get_data_frame$size)
  }

  tibble::as_tibble(get_data_frame)
}
