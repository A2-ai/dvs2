#' Set the number of threads used by DVS operations
#'
#' Controls the thread pool size for parallel file operations (add, get, status).
#' The value is stored as the `dvs.num_threads` option and synced to the
#' Rust backend before each operation.
#'
#' Can also be set via `.Rprofile` or temporarily with [withr::with_options()].
#'
#' @param threads A whole number of threads to use. `NULL` or `0` reverts to
#'   the default, automatic detection, as `--threads 0` does in the CLI. A
#'   value that is not a whole number of zero or more, `NA` included, raises a
#'   `dvs_error_argument` condition.
#'
#' @return The previous value of `dvs.num_threads` (invisibly).
#'
#' @examples
#' \dontrun{
#' set_dvs_threads(4)
#'
#' # Temporarily use 2 threads
#' withr::with_options(list(dvs.num_threads = 2), {
#'   dvs_add(paths = "big_data.csv")
#' })
#' }
#'
#' @export
set_dvs_threads <- function(threads) {
  # The Rust side checks the value and applies it now. Every operation
  # applies the option again before it runs.
  dvs_set_threads_impl(threads)
  if (!is.null(threads)) {
    threads <- as.integer(threads)
  }
  old <- getOption("dvs.num_threads")
  options(dvs.num_threads = threads)
  invisible(old)
}
