#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASE_DIR="${1:-/tmp/dvs_bench_$(date +%Y%m%d_%H%M%S)}"
STORAGE_BASE="${2:-/data/dvs/dvs_bench}"

echo "========================================"
echo "DVS Benchmark Suite"
echo "Base:    $BASE_DIR"
echo "Storage: $STORAGE_BASE"
echo "Date:    $(date)"
echo "========================================"

rm -rf "$BASE_DIR"
mkdir -p "$BASE_DIR"

# Step 1: Generate data
echo ""
echo "--- Step 1: Generate data ---"
Rscript "$SCRIPT_DIR/generate_data.R" "$BASE_DIR/data"

# Step 2: Benchmark old DVS (R package)
echo ""
echo "--- Step 2: Benchmark R package (old DVS) ---"
Rscript "$SCRIPT_DIR/bench.R" rpkg "$BASE_DIR" "$STORAGE_BASE"

# Step 3: Benchmark new DVS (CLI)
echo ""
echo "--- Step 3: Benchmark CLI (new DVS) ---"
Rscript "$SCRIPT_DIR/bench.R" cli "$BASE_DIR" "$STORAGE_BASE"

# Step 4: Comparison
echo ""
echo "--- Step 4: Head-to-head comparison ---"
Rscript -e '
base_dir <- commandArgs(trailingOnly = TRUE)[1]
rpkg <- read.csv(file.path(base_dir, "results_rpkg.csv"))
cli  <- read.csv(file.path(base_dir, "results_cli.csv"))
both <- rbind(rpkg, cli)
write.csv(both, file.path(base_dir, "results_combined.csv"), row.names = FALSE)

cat("\n=== Add: rpkg vs cli ===\n")
add_r <- rpkg[rpkg$operation == "add", c("test_type","size_mb","n_files","elapsed_sec")]
add_c <- cli[cli$operation == "add", c("test_type","size_mb","n_files","elapsed_sec")]
m <- merge(add_r, add_c, by = c("test_type","size_mb","n_files"), suffixes = c(".rpkg",".cli"))
m$ratio <- round(m$elapsed_sec.cli / m$elapsed_sec.rpkg, 2)
print(m, row.names = FALSE)

cat("\n=== Status (mean): rpkg vs cli ===\n")
stat <- both[both$operation == "status", ]
agg <- aggregate(elapsed_sec ~ tool + test_type + size_mb + n_files, data = stat, FUN = mean)
wide <- reshape(agg, direction = "wide", idvar = c("test_type","size_mb","n_files"), timevar = "tool")
names(wide) <- sub("elapsed_sec.", "", names(wide))
wide$ratio <- round(wide$cli / wide$rpkg, 2)
print(wide, row.names = FALSE)
' "$BASE_DIR"

echo ""
echo "========================================"
echo "Results:"
echo "  $BASE_DIR/results_rpkg.csv"
echo "  $BASE_DIR/results_cli.csv"
echo "  $BASE_DIR/results_combined.csv"
echo "========================================"
