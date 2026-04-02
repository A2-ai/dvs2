.onLoad <- function(libname, pkgname) {
  if (is.null(getOption("dvs.num_threads"))) {
    env_threads <- Sys.getenv("DVS_NUM_THREADS", unset = "")
    if (nzchar(env_threads)) {
      parsed <- suppressWarnings(as.integer(env_threads))
      if (!is.na(parsed) && parsed > 0L) {
        options(dvs.num_threads = parsed)
      }
    }
  } else {
    # Option was pre-set (e.g. via .Rprofile) — sync it to the env var
    .sync_threads_env()
  }
}
