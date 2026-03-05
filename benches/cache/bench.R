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

# ---- CLI runner (stderr passes through to console, exit code checked) ----
run_dvs <- function(...) {
  out <- system2("dvs", c(...), stdout = TRUE, stderr = "")
  status <- attr(out, "status")
  if (!is.null(status) && status != 0) {
    stop(sprintf("dvs %s failed (exit %d)", paste(c(...), collapse = " "), status))
  }
  invisible(out)
}

# ---- Tool wrappers ----
do_init <- switch(mode,
  rpkg = function(storage_dir) dvs_init(storage_dir),
  cli  = function(storage_dir) run_dvs("init", storage_dir)
)

do_add <- switch(mode,
  rpkg = function(pattern) dvs_add(pattern),
  cli  = function(pattern) {
    if (grepl("[*?]", pattern)) {
      run_dvs("add", "--glob", pattern)
    } else {
      run_dvs("add", pattern)
    }
  }
)

do_status <- switch(mode,
  rpkg = function() dvs_status(),
  cli  = function() run_dvs("status")
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

bench_op <- function(dir, expr) {
  owd <- setwd(dir)
  on.exit(setwd(owd))
  system.time(expr)[["elapsed"]]
}

results_list <- list()

record <- function(op, type, sz, nf, rep, elapsed) {
  results_list[[length(results_list) + 1]] <<- data.frame(
    tool = mode, operation = op, test_type = type,
    size_mb = sz, n_files = nf, replicate = rep,
    elapsed_sec = elapsed, stringsAsFactors = FALSE
  )
}

# ---- Configuration ----
single_sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)
par_sizes <- c(1, 5, 10, 50, 100)

# ===========================================================
# Phase 1: Setup — create projects, init dvs, copy data files
# ===========================================================
cat("\n--- Setup: creating projects and copying data ---\n")

single_projects <- list()
for (sz in single_sizes) {
  src <- file.path(data_dir, "single", sprintf("data_%05dmb.tab", sz))
  if (!file.exists(src)) { cat(sprintf("  SKIP single %d MB (source missing)\n", sz)); next }

  proj <- file.path(base_dir, mode, sprintf("single_%05dmb", sz))
  stor <- file.path(storage_base, mode, sprintf("single_%05dmb", sz))
  setup_project(proj, stor)
  file.copy(src, file.path(proj, basename(src)))

  single_projects[[length(single_projects) + 1]] <- list(
    proj = proj, sz = sz, file = basename(src)
  )
  cat(sprintf("  single %5d MB -> %s\n", sz, proj))
}

par_projects <- list()
for (sz in par_sizes) {
  pdata <- file.path(data_dir, "parallel", sprintf("size_%03dmb", sz))
  if (!dir.exists(pdata)) { cat(sprintf("  SKIP parallel %d MB (source missing)\n", sz)); next }

  proj <- file.path(base_dir, mode, sprintf("parallel_%03dmb", sz))
  stor <- file.path(storage_base, mode, sprintf("parallel_%03dmb", sz))
  setup_project(proj, stor)

  files <- list.files(pdata, pattern = "\\.tab$", full.names = TRUE)
  for (f in files) file.copy(f, file.path(proj, basename(f)))

  par_projects[[length(par_projects) + 1]] <- list(
    proj = proj, sz = sz, n_files = length(files)
  )
  cat(sprintf("  parallel %3d MB x %d files -> %s\n", sz, length(files), proj))
}

cat("Setup complete.\n")

# ===========================================================
# Phase 2: Benchmark add
# ===========================================================

cat("\n--- Single File Add ---\n")
for (p in single_projects) {
  cat(sprintf("  %5d MB ... ", p$sz))
  elapsed <- tryCatch(
    bench_op(p$proj, do_add(p$file)),
    error = function(e) { message("ERROR: ", e$message); NA_real_ }
  )
  if (is.na(elapsed)) next
  cat(sprintf("%.2f sec\n", elapsed))
  record("add", "single", p$sz, 1, 1, elapsed)
}

cat("\n--- Parallel Add ---\n")
for (p in par_projects) {
  cat(sprintf("  %3d MB x %d files ... ", p$sz, p$n_files))
  elapsed <- tryCatch(
    bench_op(p$proj, do_add("*.tab")),
    error = function(e) { message("ERROR: ", e$message); NA_real_ }
  )
  if (is.na(elapsed)) next
  cat(sprintf("%.2f sec\n", elapsed))
  record("add", "parallel", p$sz, p$n_files, 1, elapsed)
}

# ===========================================================
# Phase 3: Benchmark status (5 replicates)
# ===========================================================

cat("\n--- Single File Status (5 replicates) ---\n")
for (rep in 1:5) {
  cat(sprintf("  Rep %d: ", rep))
  for (p in single_projects) {
    elapsed <- tryCatch(
      bench_op(p$proj, do_status()),
      error = function(e) { message("ERROR: ", e$message); NA_real_ }
    )
    if (is.na(elapsed)) next
    cat(sprintf("%dMB=%.2fs ", p$sz, elapsed))
    record("status", "single", p$sz, 1, rep, elapsed)
  }
  cat("\n")
}

cat("\n--- Parallel Status (5 replicates) ---\n")
for (rep in 1:5) {
  cat(sprintf("  Rep %d: ", rep))
  for (p in par_projects) {
    elapsed <- tryCatch(
      bench_op(p$proj, do_status()),
      error = function(e) { message("ERROR: ", e$message); NA_real_ }
    )
    if (is.na(elapsed)) next
    cat(sprintf("%dMB=%.2fs ", p$sz, elapsed))
    record("status", "parallel", p$sz, p$n_files, rep, elapsed)
  }
  cat("\n")
}

# ---- Save results ----
results <- do.call(rbind, results_list)
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
