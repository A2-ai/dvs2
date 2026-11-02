#' Construct a dvs_bytes vector
#'
#' A thin constructor used by [dvs_add()], [dvs_status()] and [dvs_get()] to
#' tag byte-size columns so that pillar renders them as human-readable sizes
#' in tibble output. Stored as `double` to safely represent file sizes up to
#' `2^53` bytes. An R `integer` is 32 bits and would overflow at about 2 GB.
#'
#' @param x atomic numeric / integer / logical vector (incl. `NA`). `NULL`
#'   raises a `dvs_error_argument` condition.
#' @return a `dvs_bytes` object inheriting from `numeric`
#' @export
new_dvs_bytes <- function(x) {
  if (is.null(x)) {
    .miniextendr_arg_error("x", "must not be NULL", sys.call())
  }
  structure(as.double(x), class = c("dvs_bytes", "numeric"))
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

# How dvs_bytes combines with plain numbers, so vctrs::vec_c(),
# dplyr::bind_rows() and dplyr joins accept a dvs_bytes column next to a
# numeric one. Two dvs_bytes stay dvs_bytes. With a double or an integer the
# result is a plain double, as for the percent class of vctrs' S3 vignette.

#' @importFrom vctrs vec_ptype2 vec_cast
#' @export
vec_ptype2.dvs_bytes.dvs_bytes <- function(x, y, ...) new_dvs_bytes(double())
#' @export
vec_ptype2.dvs_bytes.double <- function(x, y, ...) double()
#' @export
vec_ptype2.double.dvs_bytes <- function(x, y, ...) double()
#' @export
vec_ptype2.dvs_bytes.integer <- function(x, y, ...) double()
#' @export
vec_ptype2.integer.dvs_bytes <- function(x, y, ...) double()

#' @export
vec_cast.dvs_bytes.dvs_bytes <- function(x, to, ...) x
#' @export
vec_cast.dvs_bytes.double <- function(x, to, ...) new_dvs_bytes(x)
#' @export
vec_cast.dvs_bytes.integer <- function(x, to, ...) new_dvs_bytes(x)
#' @export
vec_cast.double.dvs_bytes <- function(x, to, ...) unclass(x)

# Preserve dvs_bytes class for +/- only. For *, /, ^, %%, %/% the result is
# dimensionally not bytes (e.g. bytes * bytes would be bytes^2), so fall
# through without reclassing. Comparisons and logical ops already return
# a logical from NextMethod().
#' @export
Ops.dvs_bytes <- function(e1, e2) {
  result <- NextMethod()
  if (.Generic %in% c("+", "-")) {
    return(new_dvs_bytes(result))
  }
  # For *, /, ^, %%, %/%, comparisons, and logical ops, the result is no
  # longer dimensionally "bytes" — strip the class.
  class(result) <- setdiff(class(result), "dvs_bytes")
  result
}

#' @export
Summary.dvs_bytes <- function(..., na.rm = FALSE) {
  result <- NextMethod()
  if (.Generic %in% c("sum", "min", "max", "range")) {
    return(new_dvs_bytes(result))
  }
  result
}

# A BLAKE3 hash column: the full hex hashes, which a tibble prints as their
# first 12 characters, as `dvs history` does. `as.character()` and `==` see
# the full hashes.
new_dvs_hash <- function(x) {
  structure(as.character(x), class = c("dvs_hash", "character"))
}

#' @export
pillar_shaft.dvs_hash <- function(x, ...) {
  pillar::new_pillar_shaft_simple(substr(unclass(x), 1L, 12L), align = "left")
}

#' @export
type_sum.dvs_hash <- function(x) {
  "blake3"
}

#' @export
print.dvs_hash <- function(x, ...) {
  print(unclass(x), ...)
  invisible(x)
}
