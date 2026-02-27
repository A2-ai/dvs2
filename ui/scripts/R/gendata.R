#' Generate unique test data files
#'
#' Creates \code{n} tab-delimited files. When \code{size} is \code{NULL},
#' file \code{i} contains \code{i} rows (so every file has a unique hash).
#' When \code{size} is given, every file has \code{size} rows but a unique
#' random seed per file ensures distinct hashes.
#'
#' @param files Directory to write the files into (created if it doesn't exist).
#' @param n Number of files to generate. Defaults to \code{1}.
#' @param size Number of rows per file. If \code{NULL} (default), file \code{i}
#'   gets \code{i} rows (legacy behaviour).
#' @param data Source data frame. Defaults to \code{Theoph}.
#' @return Invisibly returns \code{NULL}. Called for its side effect of writing
#'   \code{Theoph_n_1.tab}, \code{Theoph_n_2.tab}, \ldots, \code{Theoph_n_<n>.tab}
#'   into \code{files}.
# TODO: rename `files` to `path` and check that it is one single path too
gendata <- function(files, n = 1, size = NULL, data = Theoph) {
  dir.create(files, recursive = TRUE, showWarnings = FALSE)
  for (i in seq_len(n)) {
    nrows <- if (is.null(size)) i else size + i - 1L
    rows <- rep_len(seq_len(nrow(data)), nrows)
    write.table(
      data[rows, ],
      file = file.path(files, sprintf("Theoph_n_%d.tab", i)),
      eol = "\n"
    )
  }
}
