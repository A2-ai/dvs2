#!/usr/bin/env bash
# bench_common.sh — shared infrastructure for benchmark scripts.
# Source this file; do not execute it directly.

bench_setup() {
    STORAGE="${1:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR>}"
    PROJECT_DIR="${2:?Usage: $0 <STORAGE_DIR> <PROJECT_DIR>}"
    local dvs_init_flags="${3:-}"

    SIZES_MB=(${SIZES_MB:-1 2 5 10 50})
    REPS="${REPS:-25}"
    BATCH="${BATCH:-$REPS}"

    # Resolve script dir before cd (may be relative)
    BENCH_SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

    mkdir -p "$STORAGE" "$PROJECT_DIR"
    STORAGE="$(cd "$STORAGE" && pwd)"
    PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd)"

    SCRIPT_NAME="$(basename "$0" .sh)"
    CSV="$PROJECT_DIR/${SCRIPT_NAME}_results.csv"
    HEADER_WRITTEN=0

    cd "$PROJECT_DIR"
    git init -q
    dvs init $dvs_init_flags "$STORAGE" > /dev/null
}

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

generate_files() {
    local size="$1" count="$2"
    GENERATED_FILES=()
    if [ "$count" -eq 1 ]; then
        dd if=/dev/urandom of="f.bin" bs=1048576 count="$size" 2>/dev/null
        GENERATED_FILES=("f.bin")
    else
        for i in $(seq 1 "$count"); do
            dd if=/dev/urandom of="f${i}.bin" bs=1048576 count="$size" 2>/dev/null
            GENERATED_FILES+=("f${i}.bin")
        done
    fi
}

shuffle_work() {
    WORK=()
    for size in "${SIZES_MB[@]}"; do
        for rep in $(seq 1 "$REPS"); do
            WORK+=("${size},${rep}")
        done
    done
    mapfile -t WORK < <(printf '%s\n' "${WORK[@]}" | awk 'BEGIN{srand()}{print rand()"\t"$0}' | sort -n | cut -f2-)
}

bench_teardown() {
    cp "$CSV" "$BENCH_SCRIPT_DIR/"
    echo "Results: $BENCH_SCRIPT_DIR/$(basename "$CSV")"
}
