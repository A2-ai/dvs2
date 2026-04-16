#' @export
format.dvs_bytes <- function(x, ...) {
  vapply(unclass(x), format_byte_size, character(1))
}

#' @export
as.character.dvs_bytes <- format.dvs_bytes

#' @export
print.dvs_bytes <- function(x, ...) {
  print(format.dvs_bytes(x, ...))
  invisible(x)
}
