#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

MODE="${1:?Usage: run.sh <rpkg|cli> [base_dir] [storage_base]}"
BASE_DIR="${2:-$HOME/tmp/dvs_bench_$(date +%Y%m%d_%H%M%S)}"
STORAGE_BASE="${3:-$HOME/tmp/dvs_bench_storage}"

echo "========================================"
echo "DVS Benchmark ($MODE)"
echo "Base:    $BASE_DIR"
echo "Storage: $STORAGE_BASE"
echo "Date:    $(date)"
echo "========================================"

cleanup() {
  echo ""
  echo "Cleaning up $BASE_DIR and $STORAGE_BASE ..."
  rm -rf "$BASE_DIR" "$STORAGE_BASE"
}
trap cleanup EXIT

mkdir -p "$BASE_DIR"

# Step 1: Generate data once into script_dir/data/
DATA_DIR="$SCRIPT_DIR/data"
if ! ls "$DATA_DIR"/data_*mb.tab &>/dev/null; then
  echo ""
  echo "--- Generating data ---"
  rm -rf "$DATA_DIR"
  Rscript "$SCRIPT_DIR/generate_data.R" "$DATA_DIR"
fi

# Step 2: Copy data files into per-size project dirs (parallel)
echo ""
echo "--- Copying data into project directories ---"
pids=()
for src in "$DATA_DIR"/data_*mb.tab; do
  [ -f "$src" ] || continue
  name=$(basename "$src" .tab)
  padded=${name#data_}        # e.g. "00001mb"

  proj="$BASE_DIR/$MODE/bench_${padded}"
  mkdir -p "$proj"
  cp --reflink=auto "$src" "$proj/" &
  pids+=($!)
  echo "  $name -> $proj (background)"
done

# Wait for all copies
for pid in "${pids[@]}"; do
  wait "$pid"
done
echo "Copies complete."

# Step 3: Benchmark
echo ""
echo "--- Benchmark ($MODE) ---"
Rscript "$SCRIPT_DIR/bench.R" "$MODE" "$BASE_DIR" "$STORAGE_BASE"

echo ""
echo "========================================"
echo "Results: $SCRIPT_DIR/benches/cache/results_${MODE}.csv"
echo "========================================"
