#' @section NULL and NA in arguments:
#' `NULL` in an optional argument means the argument was not given, and its
#' default applies. For example `root_dir = NULL` in [dvs_init()] is the
#' working directory, and `message = NULL` in [dvs_add()] adds no message.
#' `NA` is never a value in an argument of [dvs_init()], [dvs_add()],
#' [dvs_status()], [dvs_get()], [set_dvs_threads()] or [set_dvs_log_level()].
#' An `NA` there raises a `dvs_error_argument` condition, also a `dvs_error`,
#' that names the call. [new_dvs_bytes()] takes `NA` for an unknown size.
#' @keywords internal
"_PACKAGE"

## usethis namespace: start
#' @useDynLib dvs, .registration = TRUE
# Forces R to load cli's namespace (and DLL) at package load time so that
# cli's C callables are registered before our C shim calls them via
# R_GetCCallable. Without this, cli is never loaded because we only use
# its C API, not any R-level exports.
#' @importFrom cli cli_progress_bar
## usethis namespace: end
NULL
