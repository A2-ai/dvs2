#' Coerce a vector of byte counts to a `dvs_bytes` object
#'
#' Internal helper used by `dvs_add()`, `dvs_status()`, and `dvs_get()` to
#' tag size columns with a `dvs_bytes` class that carries human-readable
#' formatting while remaining a numeric vector (so `tibble::as_tibble()`
#' accepts it). List inputs (e.g., from `dry_run = TRUE` where Rust returns
#' `Option<u64>::None`) are flattened: empty elements become `NA_real_`.
#'
#' @param x A numeric vector, or a list of length-0/length-1 numeric values.
#' @return A numeric vector with class `c("dvs_bytes", "numeric")`.
#' @export
as.dvs_bytes <- function(x) {
  if (is.list(x)) {
    x <- vapply(
      x,
      function(v) if (length(v) == 0L) NA_real_ else as.numeric(v),
      numeric(1)
    )
  }
  structure(as.numeric(x), class = c("dvs_bytes", "numeric"))
}

#' @export
format.dvs_bytes <- function(x, ...) {
  vapply(
    unclass(x),
    function(v) if (is.na(v)) NA_character_ else format_byte_size(v),
    character(1)
  )
}

#' @export
as.character.dvs_bytes <- format.dvs_bytes

#' @export
print.dvs_bytes <- function(x, ...) {
  print(format.dvs_bytes(x, ...))
  invisible(x)
}
