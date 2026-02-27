#!/usr/bin/env bash
set -euo pipefail
LOGFILE="${0##*/}"
LOGFILE="${LOGFILE%.*}" # remove last extension
# if called with args, include them in log name
if [ $# -ge 1 ]; then
  LOGFILE="${LOGFILE}_n${1}_s${2:-1e6}.log"
else
  LOGFILE="${LOGFILE}.log"
fi
exec > >(tee "$LOGFILE") 2>&1

run() { echo "$ $*"; "$@"; echo; }
tic() { TIC_START=$(perl -MTime::HiRes=time -e 'printf "%.3f", time'); }
toc() {
  local end elapsed
  end=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  elapsed=$(echo "$end - $TIC_START" | bc)
  echo "elapsed: ${elapsed}s"; echo
  TIMINGS+=("$elapsed")
}
TIMINGS=()
LABELS=()

N="${1:-100}"
SIZE="${2:-1e6}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

cd "$TMPDIR"
mkdir projectA
mkdir storage

STORAGE="$TMPDIR/storage"
PROJECT_A="$TMPDIR/projectA"

run cd "$PROJECT_A"
run git init
run mkdir data

# generate datasets into 4 subdirectories
run R -q -e "source('$SCRIPT_DIR/R/gendata.R'); gendata(files='data/add_par_print/', n=$N, size=$SIZE); gendata(files='data/add_seq_print/', n=$N, size=$SIZE); gendata(files='data/add_par_quiet/', n=$N, size=$SIZE); gendata(files='data/add_seq_quiet/', n=$N, size=$SIZE)"

run dvs init "$STORAGE"
run tree "$TMPDIR"

# parallel add — with printing
LABELS+=("add: parallel, with printing")
echo "=== dvs add (parallel, with printing) ==="
tic
run dvs add data/add_par_print/Theoph_n_*.tab --message "parallel add with printing"
toc

run cat "$STORAGE"/audit.log.jsonl
run tree "$TMPDIR"

# show .dvs metadata for first 10 files
echo "=== .dvs metadata for first 10 added files ==="
for i in $(seq 1 10); do
  f=".dvs/data/add_par_print/Theoph_n_${i}.tab.dvs"
  echo "--- $f ---"
  cat "$f"
  echo
done

# sequential add — with printing
LABELS+=("add: sequential, with printing")
echo "=== dvs add (sequential, with printing) ==="
tic
run env DVS_NUM_THREADS=1 dvs add data/add_seq_print/Theoph_n_*.tab --message "sequential add with printing"
toc

# parallel add — without printing
LABELS+=("add: parallel, no printing")
echo "=== dvs add (parallel, no printing) ==="
tic
dvs add data/add_par_quiet/Theoph_n_*.tab --message "parallel add no printing" > /dev/null 2>&1
toc

# sequential add — without printing
LABELS+=("add: sequential, no printing")
echo "=== dvs add (sequential, no printing) ==="
tic
DVS_NUM_THREADS=1 dvs add data/add_seq_quiet/Theoph_n_*.tab --message "sequential add no printing" > /dev/null 2>&1
toc

# --- status benchmarks ---

# parallel status — with printing
LABELS+=("status: parallel, with printing")
echo "=== dvs status (parallel, with printing) ==="
tic
run dvs status
toc

# parallel status — without printing
LABELS+=("status: parallel, no printing")
echo "=== dvs status (parallel, no printing) ==="
tic
dvs status > /dev/null 2>&1
toc

# sequential status — with printing
LABELS+=("status: sequential, with printing")
echo "=== dvs status (sequential, with printing) ==="
tic
run env DVS_NUM_THREADS=1 dvs status
toc

# sequential status — without printing
LABELS+=("status: sequential, no printing")
echo "=== dvs status (sequential, no printing) ==="
tic
DVS_NUM_THREADS=1 dvs status > /dev/null 2>&1
toc

# --- get benchmarks ---

# delete local data files so get has something to fetch
run rm data/add_par_print/Theoph_n_*.tab

# parallel get — with printing
LABELS+=("get: parallel, with printing")
echo "=== dvs get (parallel, with printing) ==="
tic
run dvs get --glob "data/add_par_print/Theoph_n_*.tab"
toc

rm data/add_par_print/Theoph_n_*.tab

# parallel get — without printing
LABELS+=("get: parallel, no printing")
echo "=== dvs get (parallel, no printing) ==="
tic
dvs get --glob "data/add_par_print/Theoph_n_*.tab" > /dev/null 2>&1
toc

rm data/add_par_print/Theoph_n_*.tab

# sequential get — with printing
LABELS+=("get: sequential, with printing")
echo "=== dvs get (sequential, with printing) ==="
tic
run env DVS_NUM_THREADS=1 dvs get --glob "data/add_par_print/Theoph_n_*.tab"
toc

rm data/add_par_print/Theoph_n_*.tab

# sequential get — without printing
LABELS+=("get: sequential, no printing")
echo "=== dvs get (sequential, no printing) ==="
tic
DVS_NUM_THREADS=1 dvs get --glob "data/add_par_print/Theoph_n_*.tab" > /dev/null 2>&1
toc

# summary
echo "==========================================="
echo "  Timing Summary ($N files, $SIZE rows each)"
echo "==========================================="
for i in "${!LABELS[@]}"; do
  printf "  %-40s %ss\n" "${LABELS[$i]}" "${TIMINGS[$i]}"
done
echo

exit 0
