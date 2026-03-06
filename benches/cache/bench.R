#!/usr/bin/env Rscript
# DVS Benchmark: Add + Status performance
# Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> <out_csv>
#
# Measures dvs via either the R package or CLI interface.
# Appends results to <out_csv> with an interface column (dvs1 or dvs2).
# Source data is read from script_dir/data/ (generated once by generate_data.R).

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 4) stop("Usage: Rscript bench.R <rpkg|cli> <base_dir> <storage_base> <out_csv>")

mode <- args[1]
base_dir <- args[2]
storage_base <- args[3]
out_file <- args[4]

version_label <- switch(mode, rpkg = "dvs1", cli = "dvs2")

stopifnot(mode %in% c("rpkg", "cli"))
options(scipen = 999)

cat(sprintf("\n========================================\n"))
cat(sprintf("DVS Benchmark (%s interface)\n", mode))
cat(sprintf("========================================\n"))

# ---- Preflight ----
if (mode == "rpkg") {
  library(dvs1)
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
  t0 <- proc.time()[["elapsed"]]
  expr
  proc.time()[["elapsed"]] - t0
}

results_list <- list()

record <- function(op, sz, rep, elapsed) {
  results_list[[length(results_list) + 1]] <<- data.frame(
    interface = version_label, operation = op,
    size_mb = sz, replicate = rep,
    elapsed_sec = elapsed, stringsAsFactors = FALSE
  )
}

# ===========================================================
# Phase 1: Setup — discover projects and init dvs
# ===========================================================
cat("\n--- Setup: initializing projects ---\n")

proj_root <- file.path(base_dir, mode)
proj_dirs <- sort(list.dirs(proj_root, recursive = FALSE, full.names = TRUE))

projects <- list()
for (proj in proj_dirs) {
  tabs <- list.files(proj, pattern = "\\.tab$")
  if (length(tabs) == 0) next

  sz <- as.numeric(sub("^bench_0*([0-9]+)mb$", "\\1", basename(proj)))
  stor <- file.path(storage_base, mode, basename(proj))
  setup_project(proj, stor)

  projects[[length(projects) + 1]] <- list(
    proj = proj, sz = sz, file = tabs[1]
  )
  cat(sprintf("  %5.0f MB: git init + dvs init -> %s\n", sz, proj))
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
if (length(results_list) == 0) stop("No results recorded. Check that run.sh copied data into: ", base_dir)
results <- do.call(rbind, results_list)

summaries <- list()

add_res <- results[results$operation == "add", ]
if (nrow(add_res) > 0) {
  summaries$add <- data.frame(
    interface = version_label, operation = "add",
    size_mb = add_res$size_mb,
    mean_sec = add_res$elapsed_sec,
    std_sec = NA_real_,
    n = 1L
  )
}

stat_res <- results[results$operation == "status", ]
if (nrow(stat_res) > 0) {
  summaries$status <- do.call(rbind, lapply(split(stat_res, stat_res$size_mb), function(d) {
    data.frame(
      interface = version_label, operation = "status",
      size_mb = d$size_mb[1],
      mean_sec = mean(d$elapsed_sec),
      std_sec = sd(d$elapsed_sec),
      n = nrow(d)
    )
  }))
}

summary_results <- do.call(rbind, summaries)

append <- file.exists(out_file)
write.table(summary_results, out_file, sep = ",", row.names = FALSE,
            col.names = !append, append = append)
cat(sprintf("\nResults appended to: %s\n", out_file))

# ---- Summary ----
cat(sprintf("\n--- Summary (%s) ---\n", version_label))
print(summary_results[, c("operation", "size_mb", "mean_sec", "std_sec")], row.names = FALSE)
