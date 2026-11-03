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

#' @inherit dvs_init_server_impl title description params
#' @return The project root, the folder that holds `dvs.toml`, as
#'   `project_root` in `dvs init --json`.
#'
#'   A project that exists on the server, a group you are not in and a
#'   refused login raise a `dvs_error` with the CLI's `code`.
#' @examples
#' \dontrun{
#' dvs_init_server("my-project", "https://dvs.example.com", group = "my-team")
#' }
#' @rdname dvs_init_server
#' @export
dvs_init_server <- function(
  name,
  url,
  group,
  metadata_folder_name = NULL,
  compression = c("zstd", "none")
) {
  root <- dvs_init_server_impl(
    name = name,
    url = url,
    group = group,
    metadata_folder_name = metadata_folder_name,
    compression = compression
  )
  if (is.na(root)) {
    # No token for the server yet: log in, as dvs init server does, then again
    dvs_login(url)
    root <- dvs_init_server_impl(
      name = name,
      url = url,
      group = group,
      metadata_folder_name = metadata_folder_name,
      compression = compression
    )
  }
  root
}

#' @inherit dvs_attach_impl title description params
#' @return The project root, the working directory.
#'
#'   A project the server does not have (`"project_not_found"`), a project
#'   you cannot reach (`"access_denied"`) and a refused login raise a
#'   `dvs_error` with the CLI's `code`.
#' @examples
#' \dontrun{
#' dvs_attach("my-project", "https://dvs.example.com")
#' }
#' @rdname dvs_attach
#' @export
dvs_attach <- function(name, url, metadata_folder_name = NULL) {
  root <- dvs_attach_impl(name = name, url = url, metadata_folder_name = metadata_folder_name)
  if (is.na(root)) {
    # No token for the server yet: log in, as dvs attach does, then again
    dvs_login(url)
    root <- dvs_attach_impl(name = name, url = url, metadata_folder_name = metadata_folder_name)
  }
  root
}

#' @inherit dvs_availability_impl title description params return
#' @examples
#' \dontrun{
#' status <- dvs_status()
#' dvs_availability(status$hash[!is.na(status$hash)])
#' }
#' @rdname dvs_availability
#' @export
dvs_availability <- function(hashes) {
  tibble::as_tibble(dvs_availability_impl(hashes = hashes))
}
