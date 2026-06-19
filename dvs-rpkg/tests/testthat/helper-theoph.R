# Create the parent directory so callers can pass nested paths (e.g.
# "data/raw/deep.csv"); write.csv() does not create it.
ensure_parent_dir <- function(path) {
  dir.create(dirname(path), recursive = TRUE, showWarnings = FALSE)
}

write_theoph <- function(path) {
  ensure_parent_dir(path)
  data <- datasets::Theoph
  utils::write.csv(data, file = path, row.names = FALSE)
  invisible(path)
}

write_theoph_shuffled <- function(path, seed) {
  ensure_parent_dir(path)
  data <- datasets::Theoph
  set.seed(seed)
  data <- data[sample(nrow(data)), , drop = FALSE]
  utils::write.csv(data, file = path, row.names = FALSE)
  invisible(path)
}
