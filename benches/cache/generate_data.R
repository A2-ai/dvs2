#!/usr/bin/env Rscript
# Generate test data files for DVS benchmarking
# Usage: Rscript generate_data.R <output_dir>

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 1) stop("Usage: Rscript generate_data.R <output_dir>")
output_dir <- args[1]

NCOL <- 10
CHUNK_ROWS <- 500000

write_tab <- function(df, path, append = FALSE) {
  write.table(df, path, sep = "\t", row.names = FALSE, eol = "\n",
              col.names = !append, append = append)
}

calibrate <- function() {
  tf <- tempfile(fileext = ".tab")
  on.exit(unlink(tf))
  set.seed(42)
  df <- data.frame(matrix(rnorm(10000 * NCOL), ncol = NCOL))
  write.table(df, tf, sep = "\t", row.names = FALSE, eol = "\n",
              col.names = FALSE)
  file.size(tf) / 10000
}

generate_file <- function(path, target_mb, bpr) {
  target_rows <- ceiling(target_mb * 1024^2 / bpr)
  rows_written <- 0
  chunk <- 0

  while (rows_written < target_rows) {
    chunk <- chunk + 1
    n <- min(CHUNK_ROWS, target_rows - rows_written)
    set.seed(as.integer(target_mb * 100 + chunk))
    df <- data.frame(matrix(rnorm(n * NCOL), ncol = NCOL))
    write_tab(df, path, append = (rows_written > 0))
    rows_written <- rows_written + n
    if (target_mb >= 100) {
      cat(sprintf("\r    %s: %.0f%%", basename(path),
                  100 * rows_written / target_rows))
    }
  }
  if (target_mb >= 100) cat("\n")

  actual_mb <- file.size(path) / 1024^2
  cat(sprintf("  %s: target=%d MB, actual=%.1f MB (%d rows)\n",
              basename(path), target_mb, actual_mb, rows_written))
  invisible(actual_mb)
}

# ---- Main ----
sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)

total_gb <- sum(sizes) / 1024
cat(sprintf("DVS Benchmark: Data Generation\n"))
cat(sprintf("This will generate ~%.0f GB of test data in %s\n", total_gb, output_dir))

bpr <- calibrate()
cat(sprintf("Calibrated: %.1f bytes/row (%d cols)\n\n", bpr, NCOL))

dir.create(output_dir, recursive = TRUE, showWarnings = FALSE)

for (sz in sizes) {
  generate_file(file.path(output_dir, sprintf("data_%05dmb.tab", sz)), sz, bpr)
}

cat("\nData generation complete.\n")
