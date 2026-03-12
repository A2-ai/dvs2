#!/usr/bin/env bash
set -euo pipefail

# Run all 8 benchmark scripts sequentially, storing results in a
# commit-named directory. Cleans up temp dirs between runs.
#
# Usage: ./run_all.sh <STORAGE_DIR> <PROJECT_DIR> [RESULT_DEST_DIR]
# Default RESULT_DEST_DIR: benchmark_log/<current git commit hash>
#
# Environment variables (passed through to bench scripts):
#   SIZES_MB  - space-separated file sizes in MB (default: "1 2 5 10 50")
#   REPS      - repetitions per size (default: 25)
#   BATCH     - files per dvs add in batch scripts (default: REPS)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STORAGE="${1:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR> [RESULT_DEST_DIR]}"
PROJECT_DIR="${2:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR> [RESULT_DEST_DIR]}"
COMMIT=$(git -C "$SCRIPT_DIR" rev-parse HEAD)
DEST="${3:-$SCRIPT_DIR}/$COMMIT"

export SIZES_MB="${SIZES_MB:-1 2 5 10 50}"
export REPS="${REPS:-25}"
export BATCH="${BATCH:-$REPS}"

SCRIPTS=(
  bench_single_serial
  bench_single_random
  bench_batch_serial
  bench_batch_random
  bench_single_serial_no_compression
  bench_single_random_no_compression
  bench_batch_serial_no_compression
  bench_batch_random_no_compression
)

mkdir -p "$STORAGE" "$PROJECT_DIR" "$DEST"
STORAGE="$(cd "$STORAGE" && pwd)"
PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"
DEST="$(cd "$DEST" && pwd)"
echo "Commit:    $COMMIT"
echo "Results:   $DEST"
echo "SIZES_MB:  $SIZES_MB"
echo "REPS:      $REPS"
echo "BATCH:     $BATCH"
echo ""

for s in "${SCRIPTS[@]}"; do
  echo "=== $s ==="
  SIZES_MB="$SIZES_MB" REPS="$REPS" BATCH="$BATCH" bash "$SCRIPT_DIR/${s}.sh" "$STORAGE" "$PROJECT_DIR"
  if [ -f "$SCRIPT_DIR/${s}_results.csv" ]; then
    mv "$SCRIPT_DIR/${s}_results.csv" "$DEST/"
  fi
  rm -rf "$PROJECT_DIR"
done

echo ""
echo "Done. $(ls "$DEST"/*.csv 2>/dev/null | wc -l | tr -d ' ') CSVs in $DEST"
