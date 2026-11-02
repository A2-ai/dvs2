#' @inherit dvs_info_impl title description return
#' @examples
#' \dontrun{
#' info <- dvs_info()
#' info$access
#' }
#' @rdname dvs_info
#' @export
dvs_info <- function() {
  info <- dvs_info_impl()
  # An empty list from serde, and character(0) is the type of a group list
  if (!is.null(info$groups)) {
    info$groups <- as.character(unlist(info$groups))
  }
  info
}
