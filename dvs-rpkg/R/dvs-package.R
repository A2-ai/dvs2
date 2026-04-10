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
