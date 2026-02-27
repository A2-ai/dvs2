#' Source a script and capture echoed output in a file
#'
#' Runs [source()] while redirecting standard output and messages to `out`.
#' If `out` already exists, a warning is issued and the file is overwritten
#' (truncated).
#'
#' @param script Character scalar. Path to the script passed to [source()].
#' @param out Character scalar. Output file path. Defaults to `paste0(script, ".out")`.
#' @param echo Logical. Passed to [source()]. Defaults to `TRUE`.
#' @param max.deparse.length Numeric/integer. Passed to [source()].
#'   Use `Inf` to avoid `"[TRUNCATED]"` markers.
#' @param split Logical. Passed to [sink()] for standard output.
#'   If `TRUE`, output is written to both console and file.
#' @param ... Additional arguments passed to [source()].
#'
#' @return Invisibly returns the value from [source()].
#'
#' @examples
#' \dontrun{
#'   source_to_file("script.R", out = "script.R.out")
#'   source_to_file("script.R", out = "script.R.out", split = TRUE)
#' }
source_to_file <- function(
  script,
  out = paste0(script, ".out"),
  echo = TRUE,
  max.deparse.length = Inf,
  split = FALSE,
  ...
) {
  if (file.exists(out)) {
    warning(
      sprintf("'%s' already exists and will be overwritten.", out),
      call. = FALSE
    )
  }

  con <- file(out, open = "wt") # truncate/overwrite
  out_n <- sink.number()
  msg_n <- sink.number(type = "message")

  sink(con, split = split)
  sink(con, type = "message")

  on.exit(
    {
      while (sink.number(type = "message") > msg_n) {
        sink(type = "message")
      }
      while (sink.number() > out_n) {
        sink()
      }
      close(con)
    },
    add = TRUE
  )

  invisible(source(
    script,
    echo = echo,
    max.deparse.length = max.deparse.length,
    ...
  ))
}
