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

#' Set the log level for DVS Rust-core messages
#'
#' Controls which log messages from the Rust core are routed to R's console.
#' `error` and `warn` messages go to stderr via `REprintf`; `info`, `debug`,
#' and `trace` messages go to stdout via `Rprintf`. The default level at
#' package load is `"info"`.
#'
#' @param level Character string giving the desired log level. One of
#'   `"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"`, `"off"`.
#'
#' @return Called for its side effect; returns `NULL` invisibly.
#'
#' @examples
#' \dontrun{
#' # Show debug-level messages from the Rust core
#' set_dvs_log_level("debug")
#'
#' # Suppress all log output
#' set_dvs_log_level("off")
#'
#' # Restore default
#' set_dvs_log_level("info")
#' }
#'
#' @export
set_dvs_log_level <- function(level) {
  level <- match.arg(level, c("error", "warn", "info", "debug", "trace", "off"))
  dvs_set_log_level_impl(level)
  invisible(NULL)
}
