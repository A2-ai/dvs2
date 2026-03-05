#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

MODE="${1:?Usage: run.sh <rpkg|cli> [base_dir] [storage_base]}"
BASE_DIR="${2:-/tmp/dvs_bench_$(date +%Y%m%d_%H%M%S)}"
STORAGE_BASE="${3:-/data/dvs/dvs_bench}"

echo "========================================"
echo "DVS Benchmark ($MODE)"
echo "Base:    $BASE_DIR"
echo "Storage: $STORAGE_BASE"
echo "Date:    $(date)"
echo "========================================"

mkdir -p "$BASE_DIR"

# Step 1: Generate data (skip if already present)
DATA_DIR="$BASE_DIR/data"
if [ -d "$DATA_DIR" ]; then
  echo ""
  echo "--- Data directory exists, skipping generation ---"
else
  echo ""
  echo "--- Step 1: Generate data ---"
  Rscript "$SCRIPT_DIR/generate_data.R" "$DATA_DIR"
fi

# Step 2: Benchmark
echo ""
echo "--- Step 2: Benchmark ($MODE) ---"
Rscript "$SCRIPT_DIR/bench.R" "$MODE" "$BASE_DIR" "$STORAGE_BASE"

echo ""
echo "========================================"
echo "Results: $BASE_DIR/results_${MODE}.csv"
echo "========================================"
