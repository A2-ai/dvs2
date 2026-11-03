#' @section NULL and NA in arguments:
#' `NULL` in an optional argument means the argument was not given, and the
#' dvs default applies, as when the argument is left out. For example
#' `root_dir = NULL` in [dvs_init()] is the working directory, and
#' `message = NULL` in [dvs_add()] adds no message. `character(0)` in `paths`
#' or `status` is the same as `NULL`.
#' `NA` is never a value in an argument of [dvs_init()], [dvs_add()],
#' [dvs_status()], [dvs_get()], [dvs_result_info()], [set_dvs_threads()],
#' [set_dvs_log_level()] or [format_byte_size()].
#' An `NA` there raises a `dvs_error_argument` condition, also a `dvs_error`,
#' that names the call. A bad value in the `dvs.num_threads` or `dvs.on_error`
#' option raises the same condition, and its message names the option.
#' [new_dvs_bytes()] takes `NA` for an unknown size and refuses `NULL`.
#' @section Errors:
#' A command that fails raises a `dvs_error` condition. Its `code` is the code
#' the CLI's JSON gives the same failure, for example `"not_in_project"`,
#' `"no_files_selected"`, `"auth_required"`, `"access_denied"` or
#' `"server_unavailable"`, and its message is the CLI's. A `dvs.toml` that does
#' not load raises a `dvs_config_error`, also a `dvs_error`, with `code`
#' `"invalid_configuration"`. An argument error is a `dvs_error_argument` with
#' the argument in `param`. A file that fails in [dvs_add()], [dvs_status()] or
#' [dvs_get()] raises nothing: its row has the code.
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
