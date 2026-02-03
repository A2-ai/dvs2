#' Retrieve data files from `dvs` storage
#' 
#' @param files character vector containing file paths
#' 
#' @inheritDotParams fs::dir_ls 
#' @export
dvs_get_glob <- function(path = NULL, glob = NULL, ...) {

  with_files <- if (!is.null(files)) {
    dvs_get(files = path)
  } else {
    NULL
  }
  
  with_glob <- if (!is.null(glob)) {
    dvs_get(fs::dir_ls(path = path, glob = glob, ...))
  } else {
    NULL
  }

  rbind(
    with_files,
    with_glob
  )
}