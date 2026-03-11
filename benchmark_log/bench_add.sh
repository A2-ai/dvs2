#!/usr/bin/env bash
set -euo pipefail

# Benchmark dvs add -vvv across different file sizes.
#
# Usage: ./bench_add.sh [STORAGE_PATH]
# Default storage: /data/dvs/benchmark-mossa-dvs-test

STORAGE="${1:-/data/dvs/benchmark-mossa-dvs-test}"
SIZES_MB=(1 2 5 10 50)
REPS=25

# Project directory
PROJECT_DIR="$(mktemp -d)/prj-dvs2"
OUTPUT_DIR="$PROJECT_DIR/benchmark_output"
CSV="$OUTPUT_DIR/results.csv"

mkdir -p "$PROJECT_DIR" "$OUTPUT_DIR"
echo "Project dir: $PROJECT_DIR"
echo "Storage:     $STORAGE"
echo "Sizes (MB):  ${SIZES_MB[*]}"
echo "Reps:        $REPS"
echo ""

# Init project
cd "$PROJECT_DIR"
git init -q
dvs init "$STORAGE" > /dev/null 2>&1

# CSV header
echo "size_mb,rep,hash_ms,store_ms,total_file_ms,total_add_ms" > "$CSV"

for size in "${SIZES_MB[@]}"; do
    echo "=== ${size} MB ==="
    for rep in $(seq 1 "$REPS"); do
        file="testfile_${size}MB.bin"

        # Generate fresh random content each rep (unique hash -> full pipeline)
        dd if=/dev/urandom of="$PROJECT_DIR/$file" bs=1048576 count="$size" 2>/dev/null

        # Remove metadata so the add is a full operation.
        # Storage blob from previous rep has a different hash, so no conflict.
        rm -f "$PROJECT_DIR/.dvs/${file}.dvs"

        # Capture stderr for the log
        LOG_FILE="$OUTPUT_DIR/size_${size}MB_rep_$(printf '%02d' "$rep").txt"
        dvs add -vvv "$file" > /dev/null 2> "$LOG_FILE" || true

        # Find and read the timing CSV produced by dvs
        TIMING_CSV=$(ls "$PROJECT_DIR"/dvs-timings-*.csv 2>/dev/null | head -1 || true)
        if [ -n "$TIMING_CSV" ] && [ -s "$TIMING_CSV" ]; then
            # Extract step timings from the CSV (skip header)
            hash_ms=$(awk -F, '$7 == "hash" { print $8 }' "$TIMING_CSV" | head -1)
            store_ms=$(awk -F, '$7 == "backend_store" { print $8 }' "$TIMING_CSV" | head -1)
            total_file_ms=$(awk -F, '$7 == "add_file_total" { print $8 }' "$TIMING_CSV" | head -1)
            total_add_ms=$(awk -F, '$7 == "add_total" { print $8 }' "$TIMING_CSV" | head -1)
            rm -f "$TIMING_CSV"
        else
            hash_ms="NA"; store_ms="NA"; total_file_ms="NA"; total_add_ms="NA"
        fi

        : "${hash_ms:=NA}" "${store_ms:=NA}" "${total_file_ms:=NA}" "${total_add_ms:=NA}"

        echo "${size},${rep},${hash_ms},${store_ms},${total_file_ms},${total_add_ms}" >> "$CSV"
        printf "  rep %2d/%d  hash=%-12s store=%-12s total=%-12s\n" \
            "$rep" "$REPS" "${hash_ms}ms" "${store_ms}ms" "${total_file_ms}ms"
    done
    echo ""
done

echo "Results: $CSV"
echo "Logs:    $OUTPUT_DIR/"
echo ""

# Summary: compute per-size averages
echo "=== Summary (mean) ==="
printf "%-8s  %-12s  %-12s  %-12s  %-12s\n" "size_mb" "hash_ms" "store_ms" "file_ms" "add_ms"
printf "%-8s  %-12s  %-12s  %-12s  %-12s\n" "-------" "-------" "--------" "-------" "------"
awk -F, 'NR > 1 && $3 != "NA" {
    s = $1
    h[s] += $3; st[s] += $4; tf[s] += $5; ta[s] += $6; n[s]++
}
END {
    for (s in n)
        printf "%-8s  %-12.3f  %-12.3f  %-12.3f  %-12.3f\n", s, h[s]/n[s], st[s]/n[s], tf[s]/n[s], ta[s]/n[s]
}' "$CSV" | sort -n
