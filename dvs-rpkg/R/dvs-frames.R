# Results from Rust come back as plain data frames or lists. These helpers give
# them the same shape every time: the columns of a spec, in its order, with its
# types, also when nothing matched (a typed zero-row tibble). Sizes are dvs_bytes,
# times POSIXct in UTC.

# Column types: chr, int, lgl, bytes (dvs_bytes), time (POSIXct UTC).
dvs_spec <- list(
  projects = c(
    name = "chr", your_role = "chr", locked = "lgl", locked_by = "chr",
    locked_at = "time", lock_reason = "chr", files = "int", size = "bytes",
    stored_size = "bytes", last_activity_at = "time", created_at = "time",
    created_by = "chr"
  ),
  grants = c(group = "chr", role = "chr", removed = "lgl", group_id = "chr"),
  files = c(
    path = "chr", size = "bytes", stored_size = "bytes", hash = "chr",
    uploaded_at = "time", uploaded_by = "chr", branch = "chr", commit = "chr",
    git_state = "chr", versions = "int", first_uploaded_at = "time",
    yanked = "lgl"
  ),
  versions = c(
    hash = "chr", latest = "lgl", size = "bytes", stored_size = "bytes",
    first_uploaded_at = "time", last_uploaded_at = "time", uploads = "int",
    downloads = "int", yanked = "lgl", other_paths = "int"
  ),
  activity = c(
    at = "time", kind = "chr", user = "chr", global_admin = "lgl", path = "chr",
    hash = "chr", size = "bytes", stored_size = "bytes", branch = "chr",
    commit = "chr", git_state = "chr", message = "chr", reason = "chr",
    batch = "chr", client = "chr", detail = "chr", id = "chr"
  ),
  remote_status = c(
    path = "chr", remote_status = "chr", on_server = "lgl", yanked = "lgl",
    yank_reason = "chr", newer_uploads = "int", latest_hash = "chr",
    latest_at = "time", latest_by = "chr", latest_branch = "chr"
  )
)

# RFC 3339 times, as the server and the dvs crate write them, to POSIXct UTC.
dvs_time <- function(x) {
  x <- as.character(x)
  as.POSIXct(sub("Z$", "", x), format = "%Y-%m-%dT%H:%M:%OS", tz = "UTC")
}

dvs_cast <- function(x, type) {
  switch(type,
    chr = as.character(x),
    int = as.integer(x),
    lgl = as.logical(x),
    bytes = new_dvs_bytes(as.double(x)),
    time = dvs_time(x),
    stop("unknown column type ", type)
  )
}

# A value from a serialized list: NULL (an unset field) becomes NA of the type.
dvs_scalar <- function(x, type) {
  if (is.null(x) || length(x) == 0L) x <- NA
  dvs_cast(unlist(x), type)
}

# `x`, a data frame or a list of columns, as a tibble with the spec's columns.
# A column missing from `x` (no rows, or never set) is all NA.
dvs_frame <- function(x, spec) {
  n <- if (is.data.frame(x)) nrow(x) else max(c(0L, lengths(x)))
  cols <- lapply(names(spec), function(name) {
    col <- x[[name]]
    if (is.list(col)) col <- unlist(col)
    if (is.null(col) || length(col) == 0L) col <- rep(NA, n)
    dvs_cast(col, spec[[name]])
  })
  names(cols) <- names(spec)
  tibble::as_tibble(cols)
}

# Every page of a list from a Rust pager, stopping at `max_rows`, as a tibble.
dvs_collect <- function(pager, spec, max_rows = Inf, call = sys.call(-1)) {
  force(call)
  pages <- list()
  n <- 0
  while (n < max_rows && !dvs_pager_done_impl(pager)) {
    page <- dvs_call(dvs_pager_next_impl(pager), call = call)
    if (nrow(page) > 0L) {
      pages[[length(pages) + 1L]] <- page
      n <- n + nrow(page)
    }
  }
  rows <- if (length(pages) > 0L) do.call(rbind, pages) else data.frame()
  out <- dvs_frame(rows, spec)
  if (n > max_rows || (n == max_rows && !dvs_pager_done_impl(pager))) {
    out <- out[seq_len(max_rows), ]
    fun <- if (is.call(call)) paste0(deparse(call[[1L]]), "()") else "This function"
    warning(
      sprintf(
        "%s stopped at max_rows = %s rows; there are more. Narrow the search, or raise max_rows.",
        fun, format(max_rows, big.mark = ",", scientific = FALSE)
      ),
      call. = FALSE
    )
  }
  out
}

# A date or time argument, as the server takes it: YYYY-MM-DD or RFC 3339.
dvs_time_arg <- function(x, name, call = sys.call(-1)) {
  force(call)
  if (is.null(x)) {
    return(NULL)
  }
  if (length(x) != 1L || is.na(x)) {
    dvs_abort("invalid_argument", sprintf("`%s` must be a single date or time", name), call = call)
  }
  if (inherits(x, "POSIXt")) {
    return(dvs_rfc3339(x))
  }
  if (inherits(x, "Date")) {
    return(format(x, "%Y-%m-%d"))
  }
  if (is.character(x)) {
    return(x)
  }
  dvs_abort(
    "invalid_argument",
    sprintf("`%s` must be a Date, a POSIXct, or a string such as \"2026-10-01\"", name),
    call = call
  )
}

# A time as RFC 3339 in UTC, to the microsecond (the server's precision).
dvs_rfc3339 <- function(x) {
  seconds <- as.numeric(as.POSIXct(x))
  whole <- floor(seconds)
  micros <- round((seconds - whole) * 1e6)
  if (micros >= 1e6) {
    whole <- whole + 1
    micros <- 0
  }
  stamp <- format(as.POSIXct(whole, origin = "1970-01-01", tz = "UTC"), "%Y-%m-%dT%H:%M:%S", tz = "UTC")
  if (micros > 0) sprintf("%s.%06dZ", stamp, as.integer(micros)) else paste0(stamp, "Z")
}

# A size argument: bytes, or a string such as "100MiB".
dvs_size_arg <- function(x, name, call = sys.call(-1)) {
  force(call)
  if (is.null(x)) {
    return(NULL)
  }
  if (length(x) != 1L || is.na(x)) {
    dvs_abort("invalid_argument", sprintf("`%s` must be a single size", name), call = call)
  }
  if (is.numeric(x)) {
    return(format(round(unclass(x)), scientific = FALSE, trim = TRUE))
  }
  if (!is.character(x)) {
    dvs_abort("invalid_argument", sprintf("`%s` must be a number of bytes, or a size such as \"100MiB\"", name), call = call)
  }
  x
}

# A character argument of any length; NULL as character(0).
dvs_chr_arg <- function(x) {
  if (is.null(x)) character(0) else as.character(x)
}

dvs_format_time <- function(x) {
  if (length(x) == 0L || is.na(x)) {
    return(NA_character_)
  }
  format(x, "%Y-%m-%d %H:%M UTC", tz = "UTC")
}
