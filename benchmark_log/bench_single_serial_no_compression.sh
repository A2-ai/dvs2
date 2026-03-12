#!/usr/bin/env bash
set -euo pipefail

# Benchmark: single file per dvs add, sizes in order. No compression.

source "$(dirname "$0")/bench_common.sh"
bench_setup "$1" "$2" --no-compression

for size in "${SIZES_MB[@]}"; do
    for rep in $(seq 1 "$REPS"); do
        generate_files "$size" 1
        dvs add -vvv "${GENERATED_FILES[@]}" > /dev/null || true
        drain_timing "$size" "$rep"
        printf "  %3dMB  rep %2d/%d\n" "$size" "$rep" "$REPS"
    done
done

bench_teardown
