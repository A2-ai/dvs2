#' @keywords internal
"_PACKAGE"

## usethis namespace: start
#' @useDynLib dvs, .registration = TRUE
## usethis namespace: end
NULL

.onLoad <- function(libname, pkgname) {
  # Force cli's shared library to load so its C callables
  # (used by cli_progress_shim.c via R_GetCCallable) are registered.
  requireNamespace("cli", quietly = TRUE)
}
