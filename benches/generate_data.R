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
  write_tab(df, tf)
  file.size(tf) / 10000
}

generate_file <- function(path, target_mb, bpr, file_idx = 1) {
  target_rows <- ceiling(target_mb * 1024^2 / bpr)
  rows_written <- 0
  chunk <- 0

  while (rows_written < target_rows) {
    chunk <- chunk + 1
    n <- min(CHUNK_ROWS, target_rows - rows_written)
    set.seed(as.integer((target_mb * 1000 + file_idx) * 100 + chunk))
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
cat("DVS Benchmark: Data Generation\n")

bpr <- calibrate()
cat(sprintf("Calibrated: %.1f bytes/row (%d cols)\n\n", bpr, NCOL))

# Single files: 1, 5, 10, 50, 100, 500, 1000, 10000 MB
single_sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)
single_dir <- file.path(output_dir, "single")
dir.create(single_dir, recursive = TRUE, showWarnings = FALSE)

cat("Single files:\n")
for (sz in single_sizes) {
  generate_file(file.path(single_dir, sprintf("data_%05dmb.tab", sz)), sz, bpr)
}

# Parallel files: 20 files each of 1, 5, 10, 50, 100 MB
par_sizes <- c(1, 5, 10, 50, 100)
cat("\nParallel files (20 each):\n")
for (sz in par_sizes) {
  pdir <- file.path(output_dir, "parallel", sprintf("size_%03dmb", sz))
  dir.create(pdir, recursive = TRUE, showWarnings = FALSE)
  cat(sprintf("\n  %d MB x 20:\n", sz))
  for (i in 1:20) {
    generate_file(file.path(pdir, sprintf("data_%02d.tab", i)), sz, bpr,
                  file_idx = i)
  }
}

cat("\nData generation complete.\n")
