as_dvs_bytes <- function(x) {
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
