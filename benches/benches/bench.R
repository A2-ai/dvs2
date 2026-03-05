#!/usr/bin/env Rscript
# DVS Benchmark: Add + Status performance
# Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> [data_dir]
#
# rpkg = old DVS R package (library(dvs))
# cli  = new DVS CLI binary

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 3) stop("Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> [data_dir]")

mode <- args[1]
base_dir <- args[2]
storage_base <- args[3]
data_dir <- if (length(args) >= 4) args[4] else file.path(base_dir, "data")

stopifnot(mode %in% c("rpkg", "cli"))

cat(sprintf("\n========================================\n"))
cat(sprintf("DVS Benchmark: %s\n", toupper(mode)))
cat(sprintf("========================================\n"))

# ---- Preflight ----
if (mode == "rpkg") {
  library(dvs)
} else {
  if (system2("dvs", "--version", stdout = FALSE, stderr = FALSE) != 0) {
    stop("dvs CLI not found in PATH")
  }
}

# ---- Tool wrappers ----
do_init <- switch(mode,
  rpkg = function(storage_dir) dvs_init(storage_dir),
  cli  = function(storage_dir) {
    system2("dvs", c("init", storage_dir), stdout = TRUE, stderr = TRUE)
  }
)

do_add <- switch(mode,
  rpkg = function(pattern) dvs_add(pattern),
  cli  = function(pattern) {
    if (grepl("[*?]", pattern)) {
      system2("dvs", c("add", "--glob", pattern), stdout = TRUE, stderr = TRUE)
    } else {
      system2("dvs", c("add", pattern), stdout = TRUE, stderr = TRUE)
    }
  }
)

do_status <- switch(mode,
  rpkg = function() dvs_status(),
  cli  = function() system2("dvs", c("status"), stdout = TRUE, stderr = TRUE)
)

# ---- Helpers ----
setup_project <- function(proj_dir, storage_dir) {
  dir.create(proj_dir, recursive = TRUE, showWarnings = FALSE)
  dir.create(dirname(storage_dir), recursive = TRUE, showWarnings = FALSE)
  system2("git", c("init", proj_dir), stdout = FALSE, stderr = FALSE)
  owd <- setwd(proj_dir)
  on.exit(setwd(owd))
  do_init(storage_dir)
}

link_or_copy <- function(from, to) {
  if (!file.link(from, to)) file.copy(from, to)
}

results <- data.frame(
  tool = character(), operation = character(), test_type = character(),
  size_mb = integer(), n_files = integer(), replicate = integer(),
  elapsed_sec = numeric(), stringsAsFactors = FALSE
)

record <- function(op, type, sz, nf, rep, elapsed) {
  results <<- rbind(results, data.frame(
    tool = mode, operation = op, test_type = type,
    size_mb = sz, n_files = nf, replicate = rep,
    elapsed_sec = elapsed, stringsAsFactors = FALSE
  ))
}

# ---- Single file sizes ----
single_sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)

# ---- 1. Single file add ----
cat("\n--- Single File Add ---\n")
for (sz in single_sizes) {
  src <- file.path(data_dir, "single", sprintf("data_%05dmb.tab", sz))
  if (!file.exists(src)) { cat(sprintf("  SKIP %d MB\n", sz)); next }

  proj <- file.path(base_dir, mode, sprintf("single_%05dmb", sz))
  stor <- file.path(storage_base, mode, sprintf("single_%05dmb", sz))
  setup_project(proj, stor)

  link_or_copy(src, file.path(proj, basename(src)))
  setwd(proj)

  cat(sprintf("  %5d MB ... ", sz))
  elapsed <- system.time(
    tryCatch(do_add(basename(src)), error = function(e) message("ERROR: ", e$message))
  )[["elapsed"]]
  cat(sprintf("%.2f sec\n", elapsed))
  record("add", "single", sz, 1, 1, elapsed)
}

# ---- 2. Single file status (5 reps, cycling through sizes) ----
cat("\n--- Single File Status (5 replicates) ---\n")
for (rep in 1:5) {
  cat(sprintf("  Rep %d: ", rep))
  for (sz in single_sizes) {
    proj <- file.path(base_dir, mode, sprintf("single_%05dmb", sz))
    if (!dir.exists(proj)) next

    setwd(proj)
    elapsed <- system.time(
      tryCatch(do_status(), error = function(e) message("ERROR"))
    )[["elapsed"]]
    cat(sprintf("%dMB=%.2fs ", sz, elapsed))
    record("status", "single", sz, 1, rep, elapsed)
  }
  cat("\n")
}

# ---- 3. Parallel add (20 files per size) ----
par_sizes <- c(1, 5, 10, 50, 100)

cat("\n--- Parallel Add (20 files each) ---\n")
for (sz in par_sizes) {
  pdata <- file.path(data_dir, "parallel", sprintf("size_%03dmb", sz))
  if (!dir.exists(pdata)) { cat(sprintf("  SKIP %d MB\n", sz)); next }

  proj <- file.path(base_dir, mode, sprintf("parallel_%03dmb", sz))
  stor <- file.path(storage_base, mode, sprintf("parallel_%03dmb", sz))
  setup_project(proj, stor)

  files <- list.files(pdata, pattern = "\\.tab$", full.names = TRUE)
  for (f in files) link_or_copy(f, file.path(proj, basename(f)))

  setwd(proj)
  cat(sprintf("  %3d MB x %d files ... ", sz, length(files)))
  elapsed <- system.time(
    tryCatch(do_add("*.tab"), error = function(e) message("ERROR: ", e$message))
  )[["elapsed"]]
  cat(sprintf("%.2f sec\n", elapsed))
  record("add", "parallel", sz, length(files), 1, elapsed)
}

# ---- 4. Parallel status (5 reps, cycling through sizes) ----
cat("\n--- Parallel Status (5 replicates) ---\n")
for (rep in 1:5) {
  cat(sprintf("  Rep %d: ", rep))
  for (sz in par_sizes) {
    proj <- file.path(base_dir, mode, sprintf("parallel_%03dmb", sz))
    if (!dir.exists(proj)) next

    setwd(proj)
    elapsed <- system.time(
      tryCatch(do_status(), error = function(e) message("ERROR"))
    )[["elapsed"]]
    cat(sprintf("%dMB=%.2fs ", sz, elapsed))
    record("status", "parallel", sz, 20, rep, elapsed)
  }
  cat("\n")
}

# ---- Save results ----
out_file <- file.path(base_dir, sprintf("results_%s.csv", mode))
write.csv(results, out_file, row.names = FALSE)
cat(sprintf("\nResults saved: %s\n", out_file))

# ---- Summary ----
cat(sprintf("\n--- %s Summary ---\n", toupper(mode)))

cat("\nAdd (seconds):\n")
add_res <- results[results$operation == "add", ]
print(add_res[, c("test_type", "size_mb", "n_files", "elapsed_sec")], row.names = FALSE)

cat("\nStatus mean ± sd (seconds):\n")
stat_res <- results[results$operation == "status", ]
if (nrow(stat_res) > 0) {
  agg <- aggregate(elapsed_sec ~ test_type + size_mb + n_files, data = stat_res,
                   FUN = function(x) sprintf("%.3f ± %.3f", mean(x), sd(x)))
  print(agg, row.names = FALSE)
}
