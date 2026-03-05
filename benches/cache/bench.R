#!/usr/bin/env Rscript
# DVS Benchmark: Add + Status performance
# Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> [data_dir]
#
# Measures dvs via either the R package or CLI interface.
# Run once per dvs install (old vs new) and compare the output CSVs.

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 3) stop("Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> [data_dir]")

mode <- args[1]
base_dir <- args[2]
storage_base <- args[3]
data_dir <- if (length(args) >= 4) args[4] else file.path(base_dir, "data")

stopifnot(mode %in% c("rpkg", "cli"))

cat(sprintf("\n========================================\n"))
cat(sprintf("DVS Benchmark (%s interface)\n", mode))
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

record <- function(op, sz, rep, elapsed) {
  results_list[[length(results_list) + 1]] <<- data.frame(
    interface = mode, operation = op,
    size_mb = sz, replicate = rep,
    elapsed_sec = elapsed, stringsAsFactors = FALSE
  )
}

# ---- Configuration ----
sizes <- c(1, 5, 10, 50, 100, 500, 1000, 10000)

# ===========================================================
# Phase 1: Setup — create projects, init dvs, copy data files
# ===========================================================
cat("\n--- Setup: creating projects and copying data ---\n")

projects <- list()
for (sz in sizes) {
  src <- file.path(data_dir, sprintf("data_%05dmb.tab", sz))
  if (!file.exists(src)) { cat(sprintf("  SKIP %d MB (source missing)\n", sz)); next }

  proj <- file.path(base_dir, mode, sprintf("bench_%05dmb", sz))
  stor <- file.path(storage_base, mode, sprintf("bench_%05dmb", sz))
  setup_project(proj, stor)
  file.copy(src, file.path(proj, basename(src)))

  projects[[length(projects) + 1]] <- list(
    proj = proj, sz = sz, file = basename(src)
  )
  cat(sprintf("  %5d MB -> %s\n", sz, proj))
}

cat("Setup complete.\n")

# ===========================================================
# Phase 2: Benchmark add
# ===========================================================

cat("\n--- Add ---\n")
for (p in projects) {
  cat(sprintf("  %5d MB ... ", p$sz))
  elapsed <- tryCatch(
    bench_op(p$proj, do_add(p$file)),
    error = function(e) { message("ERROR: ", e$message); NA_real_ }
  )
  if (is.na(elapsed)) next
  cat(sprintf("%.2f sec\n", elapsed))
  record("add", p$sz, 1, elapsed)
}

# ===========================================================
# Phase 3: Benchmark status (5 replicates)
# ===========================================================

cat("\n--- Status (5 replicates) ---\n")
for (rep in 1:5) {
  cat(sprintf("  Rep %d: ", rep))
  for (p in projects) {
    elapsed <- tryCatch(
      bench_op(p$proj, do_status()),
      error = function(e) { message("ERROR: ", e$message); NA_real_ }
    )
    if (is.na(elapsed)) next
    cat(sprintf("%dMB=%.2fs ", p$sz, elapsed))
    record("status", p$sz, rep, elapsed)
  }
  cat("\n")
}

# ---- Save results ----
results <- do.call(rbind, results_list)
out_file <- file.path(base_dir, sprintf("results_%s.csv", mode))
write.csv(results, out_file, row.names = FALSE)
cat(sprintf("\nResults saved: %s\n", out_file))

# ---- Summary ----
cat(sprintf("\n--- Summary (%s) ---\n", mode))

cat("\nAdd (seconds):\n")
add_res <- results[results$operation == "add", ]
print(add_res[, c("size_mb", "elapsed_sec")], row.names = FALSE)

cat("\nStatus mean ± sd (seconds):\n")
stat_res <- results[results$operation == "status", ]
if (nrow(stat_res) > 0) {
  agg <- aggregate(elapsed_sec ~ size_mb, data = stat_res,
                   FUN = function(x) sprintf("%.3f ± %.3f", mean(x), sd(x)))
  print(agg, row.names = FALSE)
}
