write_theoph <- function(path) {
  data <- datasets::Theoph
  utils::write.csv(data, file = path, row.names = FALSE)
  invisible(path)
}

write_theoph_shuffled <- function(path, seed) {
  data <- datasets::Theoph
  set.seed(seed)
  data <- data[sample(nrow(data)), , drop = FALSE]
  utils::write.csv(data, file = path, row.names = FALSE)
  invisible(path)
}
