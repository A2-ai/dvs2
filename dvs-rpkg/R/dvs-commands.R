#' @inheritDotParams dvs_init_impl
#' @inherit dvs_init_impl title description
#' @rdname dvs_init
#' @export
dvs_init <- function(...) {
  dvs_init_impl(...)
}

#' @inheritDotParams dvs_add_impl
#' @inherit dvs_add_impl title description
#' @rdname dvs_add
#' @export
dvs_add <- function(...) {
  dvs_add_impl(...)
}

#' @inheritDotParams dvs_status_impl
#' @inherit dvs_status_impl title description
#' @rdname dvs_status
#' @export
dvs_status <- function(...) {
  dvs_status_impl(...)
}

#' @inheritDotParams dvs_get_impl
#' @inherit dvs_get_impl title description
#' @rdname dvs_get
#' @export
dvs_get <- function(...) {
  dvs_get_impl(...)
}