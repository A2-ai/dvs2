#' Construct a dvs_bytes vector
#'
#' A thin constructor used by [dvs_add()], [dvs_status()] and [dvs_get()] to
#' tag byte-size columns so that pillar renders them as human-readable sizes
#' in tibble output. Stored as `double` to safely represent file sizes up to
#' `2^53` bytes — `integer` in R is 32 bits and would silently overflow at
#' ~2 GB.
#'
#' `x` may be:
#' * `NULL` — treated as `numeric(0)`
#' * an atomic numeric / integer / logical vector (incl. `NA`)
#' * a list of length-1 numeric vectors or `NULL`s, as produced by serde
#'   `Option<u64>` (e.g. `stored_size` in `dvs_add(dry_run = TRUE)`)
#'
#' @param x input to coerce
#' @return a `dvs_bytes` object inheriting from `numeric`
#' @export
new_dvs_bytes <- function(x) {
  if (is.null(x)) {
    x <- numeric(0)
  } else if (is.list(x)) {
    x <- vapply(
      x,
      function(v) if (is.null(v)) NA_real_ else as.double(v),
      numeric(1)
    )
  } else {
    x <- as.double(x)
  }
  structure(x, class = c("dvs_bytes", "numeric"))
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
