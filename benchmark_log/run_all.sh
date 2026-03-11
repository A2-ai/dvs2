#!/usr/bin/env bash
set -euo pipefail

# Run all 8 benchmark scripts sequentially, storing results in a
# commit-named directory. Cleans up temp dirs between runs.
#
# Usage: ./run_all.sh <STORAGE_DIR> [DEST_DIR]
# Default DEST_DIR: benchmark_log/<current git commit hash>

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STORAGE="${1:?Usage: $0 <STORAGE_DIR> [DEST_DIR]}"
COMMIT=$(git -C "$SCRIPT_DIR" rev-parse HEAD)
DEST="${2:-$SCRIPT_DIR/$COMMIT}"

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

mkdir -p "$DEST"
echo "Commit:  $COMMIT"
echo "Results: $DEST"
echo ""

for s in "${SCRIPTS[@]}"; do
  echo "=== $s ==="
  TMPDIR=$(mktemp -d)
  PROJECT_DIR="$TMPDIR/prj-dvs2"
  bash "$SCRIPT_DIR/${s}.sh" "$STORAGE" "$PROJECT_DIR" 2>&1 | tail -1
  if [ -f "$SCRIPT_DIR/${s}_results.csv" ]; then
    mv "$SCRIPT_DIR/${s}_results.csv" "$DEST/"
  fi
  rm -rf "$TMPDIR"
done

echo ""
echo "Done. $(ls "$DEST"/*.csv 2>/dev/null | wc -l | tr -d ' ') CSVs in $DEST"
