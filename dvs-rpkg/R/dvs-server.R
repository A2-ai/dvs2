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

#' @inherit dvs_login_start_impl title description params
#' @return `NULL`, invisibly.
#' @examples
#' \dontrun{
#' dvs_login()
#' dvs_login("https://dvs.example.com")
#' }
#' @rdname dvs_login
#' @export
dvs_login <- function(url = NULL) {
  login <- dvs_login_start_impl(url = url, interactive_session = interactive())
  message("Go to ", login$approve_url, " and enter the code ", login$user_code)
  # The wait sleeps in R, not in Rust, so an interrupt stops it
  attempt <- 0L
  while (!dvs_login_poll_impl(login$server, login$device_code, attempt)) {
    Sys.sleep(login$interval)
    attempt <- attempt + 1L
  }
  message("You are now logged in!")
  invisible(NULL)
}

#' @inherit dvs_logout_impl title description return
#' @examples
#' \dontrun{
#' dvs_logout()
#' }
#' @rdname dvs_logout
#' @export
dvs_logout <- function() {
  logout <- dvs_logout_impl()
  warnings <- logout$warnings
  logout$warnings <- tibble::tibble(
    code = vapply(warnings, `[[`, character(1), "code"),
    message = vapply(warnings, `[[`, character(1), "message")
  )
  logout
}
