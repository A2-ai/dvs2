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
  if (!is.null(glob)) {
    files <- unique(c(files, Sys.glob(glob)))
    glob <- NULL
  }

  use_progress <- length(files) > 1 && !isTRUE(dry_run) && interactive()
  progress_callback <- NULL

  if (use_progress) {
    pb <- progress::progress_bar$new(
      format = "  [:bar] :current/:total (:percent) eta: :eta",
      total = length(files),
      clear = FALSE
    )
    progress_callback <- function() pb$tick()
  }

  result <- dvs_add_impl(
    files = files,
    message = message,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  )

  if (requireNamespace("tibble")) {
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
  status_data_frame <-
    dvs_status_impl(current = current, absent = absent, unsynced = unsynced)
  if (requireNamespace("tibble")) {
    tibble::as_tibble(status_data_frame)
  } else {
    status_data_frame
  }
}

#' @inherit dvs_get_impl title description params
#' @rdname dvs_get
#' @export
dvs_get <- function(files = character(0), glob = NULL, dry_run = NULL) {
  use_progress <-
    length(files) > 1 && is.null(glob) && !isTRUE(dry_run) && interactive()
  progress_callback <- NULL

  if (use_progress) {
    pb <- progress::progress_bar$new(
      format = "  [:bar] :current/:total (:percent) eta: :eta",
      total = length(files),
      clear = FALSE
    )
    progress_callback <- function() pb$tick()
  }

  get_data_frame <- dvs_get_impl(
    files = files,
    glob = glob,
    dry_run = dry_run,
    progress_callback = progress_callback
  )
  if (requireNamespace("tibble")) {
    tibble::as_tibble(get_data_frame)
  } else {
    get_data_frame
  }
}
