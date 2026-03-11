#!/usr/bin/env bash
set -euo pipefail

# Benchmark: single file per dvs add, sizes randomized. No compression.

STORAGE="${1:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR>}"
PROJECT_DIR="${2:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR>}"
SIZES_MB=(${SIZES_MB:-1 2 5 10 50})
REPS="${REPS:-25}"

mkdir -p "$STORAGE" "$PROJECT_DIR"
STORAGE="$(cd "$STORAGE" && pwd)"
PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"

SCRIPT_NAME="$(basename "$0" .sh)"
CSV="$PROJECT_DIR/${SCRIPT_NAME}_results.csv"
HEADER_WRITTEN=0

cd "$PROJECT_DIR"
git init -q
dvs init --no-compression "$STORAGE" > /dev/null

drain_timing() {
    local size="$1" rep="$2"
    local f
    f=$(find "$PROJECT_DIR" -maxdepth 1 -name 'dvs-timings-*.csv' -print -quit 2>/dev/null || true)
    if [ -n "$f" ] && [ -s "$f" ]; then
        if [ "$HEADER_WRITTEN" -eq 0 ]; then
            head -1 "$f" | awk '{ print "size_mb,rep," $0 }' > "$CSV"
            HEADER_WRITTEN=1
        fi
        tail -n +2 "$f" | awk -v sz="$size" -v r="$rep" '{ print sz "," r "," $0 }' >> "$CSV"
        rm -f "$f"
    fi
}

# Build and shuffle work items
WORK=()
for size in "${SIZES_MB[@]}"; do
    for rep in $(seq 1 "$REPS"); do
        WORK+=("${size},${rep}")
    done
done
mapfile -t WORK < <(printf '%s\n' "${WORK[@]}" | awk 'BEGIN{srand()}{print rand()"\t"$0}' | sort -n | cut -f2-)

TOTAL=${#WORK[@]}
I=0
for item in "${WORK[@]}"; do
    IFS=',' read -r size rep <<< "$item"
    I=$((I + 1))
    dd if=/dev/urandom of="f.bin" bs=1048576 count="$size" 2>/dev/null
    dvs add -vvv f.bin > /dev/null || true
    drain_timing "$size" "$rep"
    printf "  [%3d/%d] %3dMB rep %2d\n" "$I" "$TOTAL" "$size" "$rep"
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cp "$CSV" "$SCRIPT_DIR/"
echo "Results: $SCRIPT_DIR/$(basename "$CSV")"
