new_dvs_bytes <- function(x) {
  if (is.null(x)) {
    x <- integer(0)
  } else if (is.list(x)) {
    x <- vapply(
      x,
      function(v) if (is.null(v)) NA_integer_ else as.integer(v),
      integer(1)
    )
  }
  structure(x, class = c("dvs_bytes", class(x)))
}

#' @importFrom pillar pillar_shaft new_pillar_shaft_simple
#' @export
pillar_shaft.dvs_bytes <- function(x, ...) {
  vals <- unclass(x)
  formatted <- vapply(
    vals,
    function(v) if (is.na(v)) NA_character_ else format_byte_size(v),
    character(1)
  )
  pillar::new_pillar_shaft_simple(formatted, align = "right")
}

#' @importFrom pillar type_sum
#' @export
type_sum.dvs_bytes <- function(x) {
  "bytes"
}

#' @export
Ops.dvs_bytes <- function(e1, e2) {
  result <- NextMethod()
  if (is.logical(result)) {
    return(result)
  }
  new_dvs_bytes(result)
}

#' @export
Summary.dvs_bytes <- function(..., na.rm = FALSE) {
  result <- NextMethod()
  if (.Generic %in% c("sum", "min", "max", "range", "prod")) {
    return(new_dvs_bytes(result))
  }
  result
}
