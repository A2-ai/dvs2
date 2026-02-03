#' Retrieve data files from `dvs` storage with glob filtering
#'
#' @param files character vector containing file paths
#'
#' @inheritDotParams fs::dir_ls
#' @rdname dvs_get
#' @export
dvs_get_glob <- function(...) {
  dvs_get(fs::dir_ls(...))
}

#' Add data files to `dvs` storage with glob filtering
#' 
#' @inheritDotParams fs::dir_ls
#' 
#' @rdname dvs_add
#' @export
dvs_add_glob <- function(..., message = quote(expr=)) {
  dvs_add(fs::dir_ls(...), message = message)
}
