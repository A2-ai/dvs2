#' dvs errors
#'
#' Every error from dvs is a condition of class
#' `c("dvs_error_<code>", "dvs_error", "error", "condition")`, so it can be
#' caught by its code:
#'
#' ```r
#' tryCatch(dvs_activity(), dvs_error_unauthenticated = function(e) dvs_login())
#' ```
#'
#' Its fields:
#'
#' * `code`: stable and machine-readable; branch on this. Codes from the server
#'   include `unauthenticated`, `token_expired`, `not_logged_in`,
#'   `project_not_found`, `insufficient_role` and `project_locked`. dvs gives its
#'   own errors the codes `not_in_repository`, `server_projects_only`,
#'   `not_interactive`, `invalid_argument` and `files_failed`, and anything
#'   else the code `error`.
#' * `status`: the server's HTTP status, or `NA` when no server answered.
#' * `message`: for people.
#' * `request_id`: finds the server's log line when reporting a problem, or
#'   `NA`.
#'
#' When files fail in [dvs_add()] or [dvs_get()], the error has the code
#' `files_failed` and a `results` field holding the tibble of every file's
#' result.
#'
#' @name dvs_error
NULL

new_dvs_error <- function(
  code,
  message,
  status = NA_integer_,
  request_id = NA_character_,
  call = NULL,
  ...
) {
  structure(
    list(
      message = message,
      call = call,
      code = code,
      status = as.integer(status),
      request_id = as.character(request_id),
      ...
    ),
    class = c(paste0("dvs_error_", code), "dvs_error", "error", "condition")
  )
}

dvs_abort <- function(code, message, ..., call = sys.call(-1)) {
  force(call)
  stop(new_dvs_error(code, paste(message, collapse = "\n"), ..., call = call))
}

# Evaluate a call into Rust, turning its error into a dvs_error raised from the
# exported function that made the call.
dvs_call <- function(expr, call = sys.call(-1)) {
  force(call)
  tryCatch(expr, rust_error = function(e) {
    code <- if (is.character(e$code) && length(e$code) == 1L && nzchar(e$code)) {
      e$code
    } else {
      "error"
    }
    status <- if (is.numeric(e$status) && length(e$status) == 1L) {
      e$status
    } else {
      NA_integer_
    }
    request_id <- if (is.character(e$request_id) && length(e$request_id) == 1L && nzchar(e$request_id)) {
      e$request_id
    } else {
      NA_character_
    }
    stop(new_dvs_error(
      code,
      conditionMessage(e),
      status = status,
      request_id = request_id,
      call = call
    ))
  })
}
