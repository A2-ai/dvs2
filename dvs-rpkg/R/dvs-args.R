# Checks of the exported functions' arguments, made before any call into Rust,
# so a bad argument is a dvs_error_invalid_argument naming the function the
# user called. Each takes `call`, the exported function's call; called from its
# body, the default is right.

dvs_arg_error <- function(name, what, call) {
  dvs_abort("invalid_argument", sprintf("`%s` must be %s", name, what), call = call)
}

check_string <- function(x, name, null_ok = TRUE, call = sys.call(-1)) {
  force(call)
  if (is.null(x) && null_ok) {
    return(invisible(x))
  }
  if (!is.character(x) || length(x) != 1L || is.na(x)) {
    dvs_arg_error(name, if (null_ok) "a single string, or NULL" else "a single string", call)
  }
  invisible(x)
}

check_strings <- function(x, name, call = sys.call(-1)) {
  force(call)
  if (is.null(x)) {
    return(invisible(x))
  }
  if (!is.character(x) || anyNA(x)) {
    dvs_arg_error(name, "a character vector without NA, or NULL", call)
  }
  invisible(x)
}

check_flag <- function(x, name, null_ok = TRUE, call = sys.call(-1)) {
  force(call)
  if (is.null(x) && null_ok) {
    return(invisible(x))
  }
  if (!is.logical(x) || length(x) != 1L || is.na(x)) {
    dvs_arg_error(name, if (null_ok) "TRUE, FALSE or NULL" else "TRUE or FALSE", call)
  }
  invisible(x)
}

check_max_rows <- function(x, call = sys.call(-1)) {
  force(call)
  if (!is.numeric(x) || length(x) != 1L || is.na(x) || x < 1) {
    dvs_arg_error("max_rows", "a single number of at least 1", call)
  }
  invisible(x)
}

# One of `choices`; the whole default vector means its first value.
check_choice <- function(x, name, choices, null_ok = FALSE, call = sys.call(-1)) {
  force(call)
  if (is.null(x) && null_ok) {
    return(NULL)
  }
  if (identical(x, choices)) {
    return(choices[[1L]])
  }
  if (!is.character(x) || length(x) != 1L || !x %in% choices) {
    dvs_arg_error(name, paste0("one of ", paste0('"', choices, '"', collapse = ", ")), call)
  }
  x
}

# Any of `choices`; the whole default vector means all of them.
check_choices <- function(x, name, choices, call = sys.call(-1)) {
  force(call)
  if (!is.character(x) || anyNA(x) || !all(x %in% choices)) {
    dvs_arg_error(name, paste0("some of ", paste0('"', choices, '"', collapse = ", ")), call)
  }
  x
}
