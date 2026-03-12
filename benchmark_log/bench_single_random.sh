#!/usr/bin/env bash
set -euo pipefail

# Benchmark: single file per dvs add, sizes randomized.

source "$(dirname "$0")/bench_common.sh"
bench_setup "$1" "$2"
shuffle_work

TOTAL=${#WORK[@]}
I=0
for item in "${WORK[@]}"; do
    IFS=',' read -r size rep <<< "$item"
    I=$((I + 1))
    generate_files "$size" 1
    dvs add -vvv "${GENERATED_FILES[@]}" > /dev/null || true
    drain_timing "$size" "$rep"
    printf "  [%3d/%d] %3dMB rep %2d\n" "$I" "$TOTAL" "$size" "$rep"
done

bench_teardown
