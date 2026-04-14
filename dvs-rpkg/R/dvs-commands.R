#' @inherit dvs_init_impl title description
#' @inherit dvs_init_impl params
#' @rdname dvs_init
#' @export
dvs_init <- function(
  storage_path,
  root_dir = NULL,
  group = NULL,
  metadata_folder_name = NULL,
  no_compression = NULL
) {
  dvs_init_impl(
    storage_path = storage_path,
    root_dir = root_dir,
    group = group,
    metadata_folder_name = metadata_folder_name,
    no_compression = no_compression
  )
}

#' @inherit dvs_add_impl title description params
#' @rdname dvs_add
#' @export
dvs_add <- function(
  files = character(0),
  message = NULL,
  glob = NULL,
  dry_run = NULL
) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  if (!isTRUE(dry_run)) {
    progress_callback <- ProgressBarCallback$new()
  }

  result <- dvs_add_impl(
    files = files,
    message = message,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  )

  if (requireNamespace("tibble", quietly = TRUE)) {
    tibble::as_tibble(result)
  } else {
    result
  }
}

#' @inherit dvs_status_impl title description params
#' @rdname dvs_status
#'
#'
#' @export
dvs_status <- function(current = NULL, absent = NULL, unsynced = NULL) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  status_data_frame <-
    dvs_status_impl(current = current, absent = absent, unsynced = unsynced)
  if (requireNamespace("tibble", quietly = TRUE)) {
    tibble::as_tibble(status_data_frame)
  } else {
    status_data_frame
  }
}

#' @inherit dvs_get_impl title description params
#' @rdname dvs_get
#' @export
dvs_get <- function(files = character(0), glob = NULL, dry_run = NULL) {
  dvs_set_threads_impl(getOption("dvs.num_threads"))
  progress_callback <- NULL
  if (!isTRUE(dry_run)) {
    progress_callback <- ProgressBarCallback$new()
  }

  get_data_frame <- dvs_get_impl(
    files = files,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  )
  if (requireNamespace("tibble", quietly = TRUE)) {
    tibble::as_tibble(get_data_frame)
  } else {
    get_data_frame
  }
}
