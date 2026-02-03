#' Retrieve data files from `dvs` storage
#'
#' @param files character vector containing file paths
#'
#' @inheritDotParams fs::dir_ls
#' @rdname dvs_get
#' @export
dvs_get_glob <- function(...) {
  dvs_get(fs::dir_ls(...))
}
