#' Set the number of threads used by DVS operations
#'
#' Controls the thread pool size for parallel file operations (add, get, status).
#' The value is stored as the `dvs.num_threads` option and synced to the
#' Rust backend before each operation.
#'
#' Can also be set via `.Rprofile` or temporarily with [withr::with_options()].
#'
#' @param threads A whole number of threads, or `NULL` or `0` to set no
#'   thread count. `NA`, a negative or fractional number, `TRUE`, `FALSE`, a
#'   string or more than one value raises a `dvs_error_argument` condition.
#'
#' @details
#' Each add, get and status operation takes its thread count from the first
#' rule that applies:
#'
#' 1. A thread count set here or in the `dvs.num_threads` option: that many
#'    threads, at most 32.
#' 2. The `DVS_NUM_THREADS` environment variable, when it is a whole number
#'    above 0: that many threads, at most 32. Any other value is ignored.
#' 3. Otherwise 4 threads per CPU, at most 16.
#'
#' An operation never uses more threads than it has files. `NULL` and `0` set
#' no thread count, so the operation falls through to `DVS_NUM_THREADS` or to
#' 4 threads per CPU. `--threads 0` in the CLI does the same.
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

# Applies options(dvs.num_threads) before an add, get or status. The value
# reaches the Rust side as `threads`, so a bad value is reported as the
# option, with the call of the operation the user called.
apply_dvs_threads_option <- function(frame = parent.frame()) {
  force(frame)
  rename_option_error(
    dvs_set_threads_impl(getOption("dvs.num_threads"), .call = frame),
    "threads",
    "dvs.num_threads"
  )
}

# Evaluates `expr`, whose argument `param` holds the value of
# options(<option>). A dvs_error_argument for `param` names the option in its
# message instead, and keeps `param` as the argument the option feeds. With
# `option` NULL the value did not come from an option.
rename_option_error <- function(expr, param, option) {
  if (is.null(option)) {
    return(expr)
  }
  tryCatch(expr, dvs_error_argument = function(cnd) {
    if (identical(cnd$param, param)) {
      cnd$message <- sub(
        sprintf("^[`']%s[`']", param),
        sprintf("`options(%s)`", option),
        conditionMessage(cnd)
      )
    }
    stop(cnd)
  })
}
