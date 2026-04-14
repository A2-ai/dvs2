#' Set the number of threads used by DVS operations
#'
#' Controls the thread pool size for parallel file operations (add, get, status).
#' The value is stored as the `dvs.num_threads` option and synced to the
#' Rust backend before each operation.
#'
#' Can also be set via `.Rprofile` or temporarily with [withr::with_options()].
#'
#' @param threads Integer number of threads to use. Set to `NULL` to revert to
#'   the default (automatic detection).
#'
#' @return The previous value of `dvs.num_threads` (invisibly).
#'
#' @examples
#' \dontrun{
#' set_dvs_threads(4)
#'
#' # Temporarily use 2 threads
#' withr::with_options(list(dvs.num_threads = 2), {
#'   dvs_add(files = "big_data.csv")
#' })
#' }
#'
#' @export
set_dvs_threads <- function(threads) {
  if (!is.null(threads)) {
    stopifnot(
      "'threads' must be a single positive integer" =
        is.numeric(threads) && length(threads) == 1L && !is.na(threads) && threads > 0
    )
    threads <- as.integer(threads)
  }
  old <- getOption("dvs.num_threads")
  options(dvs.num_threads = threads)
  invisible(old)
}
