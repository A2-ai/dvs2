#!/usr/bin/env bash

# Parallel thread control test.
# Compares performance of DVS add operations across:
#   1. CLI with DVS_NUM_THREADS env var
#   2. R package with set_dvs_threads() / dvs.num_threads option
#   3. R package with withr::with_options()
#
# All timing is done inside the same R process to eliminate startup
# overhead. CLI is timed wall-clock around just the dvs add call.
# Results are written to a log file in the ui/ directory.

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/scripts/helpers.sh
source "${SCRIPT_DIR}/scripts/helpers.sh"

N_FILES="${1:-20}"
FILE_SIZE="${2:-50M}"
THREADS="${3:-2}"

LOGFILE="${SCRIPT_DIR}/parallel_bench_$(date +%Y%m%d_%H%M%S).log"
R_TIMINGS="$(mktemp)"

printf '\n=== Parallel test: %d files × %s, threads=%s ===\n\n' \
  "$N_FILES" "$FILE_SIZE" "$THREADS"

# ── Generate files once ─────────────────────────────────────────────

FIXTURES="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
cd "$FIXTURES"
git init .
mkfiles "$N_FILES" "$FILE_SIZE" data/derived

# ── CLI: DVS_NUM_THREADS env var ────────────────────────────────────

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"
cd "$DVS_REPO_CLI"
git init .
dvs init "$DVS_STORAGE_CLI"
cp -r "$FIXTURES/data" "$DVS_REPO_CLI/data"

printf '\n>>> CLI add with DVS_NUM_THREADS=%s\n' "$THREADS"
TIMEFORMAT='%R'
CLI_ELAPSED="$( { time DVS_NUM_THREADS="$THREADS" dvs add data/derived/file_*.bin; } 2>&1 | tail -1 )"

# ── R: all three methods in one process ─────────────────────────────

DVS_REPO_R0="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R0="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
DVS_REPO_R1="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R1="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
DVS_REPO_R2="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R2="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
DVS_REPO_R3="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R3="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"

for d in "$DVS_REPO_R0" "$DVS_REPO_R1" "$DVS_REPO_R2" "$DVS_REPO_R3"; do
  cd "$d" && git init . && cp -r "$FIXTURES/data" "$d/data"
done

printf '\n>>> R (single process, 3 methods)\n'
tee /dev/stderr <<EOF | Rscript -
library(dvs)
library(tibble)  # pre-load so it doesn't count

timings_file <- "$R_TIMINGS"

# ── Warmup: pay one-time init costs ──

setwd("$DVS_REPO_R0")
dvs_init("$DVS_STORAGE_R0")
invisible(dvs_add(glob = "data/derived/*"))

# ── Method 1: set_dvs_threads() ──

setwd("$DVS_REPO_R1")
dvs_init("$DVS_STORAGE_R1")
set_dvs_threads(${THREADS}L)

start <- proc.time()
invisible(dvs_add(glob = "data/derived/*"))
r_set_elapsed <- (proc.time() - start)[["elapsed"]]

cat(sprintf("  R (set_dvs_threads):    %.3f s\n", r_set_elapsed))

# ── Method 2: withr::with_options() ──

setwd("$DVS_REPO_R2")
dvs_init("$DVS_STORAGE_R2")
set_dvs_threads(NULL)  # clear

env <- new.env(parent = emptyenv())
withr::with_options(list(dvs.num_threads = ${THREADS}L), {
  env\$start <- proc.time()
  invisible(dvs_add(glob = "data/derived/*"))
})
r_withr_elapsed <- (proc.time() - env\$start)[["elapsed"]]

cat(sprintf("  R (withr):              %.3f s\n", r_withr_elapsed))

# ── Method 3: DVS_NUM_THREADS env var (no R option set) ──

setwd("$DVS_REPO_R3")
dvs_init("$DVS_STORAGE_R3")
Sys.setenv(DVS_NUM_THREADS = "${THREADS}")

start <- proc.time()
dvs_add(glob = "data/derived/*")
r_env_elapsed <- (proc.time() - start)[["elapsed"]]

Sys.unsetenv("DVS_NUM_THREADS")

cat(sprintf("  R (DVS_NUM_THREADS):    %.3f s\n", r_env_elapsed))

# ── Write timings to file for the log ──

writeLines(c(
  sprintf("%.3f", r_set_elapsed),
  sprintf("%.3f", r_withr_elapsed),
  sprintf("%.3f", r_env_elapsed)
), timings_file)
EOF

# ── Read R timings ──────────────────────────────────────────────────

R_SET="$(sed -n '1p' "$R_TIMINGS")"
R_WITHR="$(sed -n '2p' "$R_TIMINGS")"
R_ENV="$(sed -n '3p' "$R_TIMINGS")"
rm -f "$R_TIMINGS"

# ── Write log ───────────────────────────────────────────────────────

cat > "$LOGFILE" <<LOGEOF
DVS Parallel Benchmark
$(date '+%Y-%m-%d %H:%M:%S')

Config
  files:   ${N_FILES} × ${FILE_SIZE}
  threads: ${THREADS}

Results (seconds)
  ┌──────────────────────────┬────────────┐
  │ Method                   │ Elapsed    │
  ├──────────────────────────┼────────────┤
$(printf '  │ CLI  (DVS_NUM_THREADS)   │ %6ss    │\n' "$CLI_ELAPSED")
$(printf '  │ R    (set_dvs_threads)   │ %6ss    │\n' "$R_SET")
$(printf '  │ R    (withr)             │ %6ss    │\n' "$R_WITHR")
$(printf '  │ R    (DVS_NUM_THREADS)   │ %6ss    │\n' "$R_ENV")
  └──────────────────────────┴────────────┘
LOGEOF

printf '\n'
cat "$LOGFILE"
printf '\nLog written to %s\n' "$LOGFILE"
