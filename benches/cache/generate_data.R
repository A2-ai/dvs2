#!/usr/bin/env Rscript
# Generate test data files for DVS benchmarking
# Usage: Rscript generate_data.R <output_dir>
#
# Uses the built-in Theoph dataset, recycled to hit target file sizes.

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 1) stop("Usage: Rscript generate_data.R <output_dir>")
output_dir <- args[1]

src <- Theoph
src_n <- nrow(src)

# Pre-format Theoph rows as tab-separated strings (once)
src_lines <- do.call(paste, c(src, sep = "\t"))
header <- paste(names(src), collapse = "\t")

# Build a reusable ~500k row chunk of pre-formatted lines
reps_per_chunk <- max(1L, 500000L %/% src_n)
chunk_lines <- rep(src_lines, reps_per_chunk)
chunk_n <- length(chunk_lines)

calibrate <- function() {
  tf <- tempfile(fileext = ".tab")
  on.exit(unlink(tf))
  cal_lines <- rep(src_lines, length.out = 10000)
  writeLines(c(header, cal_lines), tf)
  file.size(tf) / 10000
}

generate_file <- function(path, target_mb, bpr) {
  target_rows <- ceiling(target_mb * 1024^2 / bpr)
  remaining <- target_rows

  con <- file(path, "w")
  on.exit(close(con))

  writeLines(header, con)

  while (remaining >= chunk_n) {
    writeLines(chunk_lines, con)
    remaining <- remaining - chunk_n
  }

  if (remaining > 0L) {
    full <- remaining %/% src_n
    partial <- remaining %% src_n
    if (full > 0L) writeLines(rep(src_lines, full), con)
    if (partial > 0L) writeLines(src_lines[1:partial], con)
  }

  close(con)
  on.exit()

  actual_mb <- file.size(path) / 1024^2
  cat(sprintf("  %s: target=%.0f MB, actual=%.1f MB (%.0f rows)\n",
              basename(path), target_mb, actual_mb, target_rows))
  invisible(actual_mb)
}

# ---- Main ----
sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)

total_gb <- sum(sizes) / 1024
cat(sprintf("DVS Benchmark: Data Generation\n"))
cat(sprintf("This will generate ~%.0f GB of test data in %s\n", total_gb, output_dir))

bpr <- calibrate()
cat(sprintf("Calibrated: %.1f bytes/row (%d cols, Theoph)\n\n", bpr, ncol(src)))

dir.create(output_dir, recursive = TRUE, showWarnings = FALSE)

for (sz in sizes) {
  generate_file(file.path(output_dir, sprintf("data_%05dmb.tab", sz)), sz, bpr)
}

cat("\nData generation complete.\n")
