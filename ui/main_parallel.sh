#!/usr/bin/env bash

# Parallel thread control test.
# Compares performance of DVS add operations across:
#   1. CLI with DVS_NUM_THREADS env var
#   2. R package with set_dvs_threads() / dvs.num_threads option
#
# Expects identical timings for the same thread count, showing that the
# R option and the env var both reach the same Rust thread pool.

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/scripts/helpers.sh
source "${SCRIPT_DIR}/scripts/helpers.sh"

N_FILES="${1:-20}"
FILE_SIZE="${2:-50M}"
THREADS="${3:-2}"

printf '\n=== Parallel test: %d files × %s, threads=%s ===\n\n' \
  "$N_FILES" "$FILE_SIZE" "$THREADS"

# ── CLI: DVS_NUM_THREADS env var ─────────────────────────────────────

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"

cd "$DVS_REPO_CLI"
git init .
dvs init "$DVS_STORAGE_CLI"
mkfiles "$N_FILES" "$FILE_SIZE" data/derived

printf '\n>>> CLI add with DVS_NUM_THREADS=%s\n' "$THREADS"
CLI_START="$(perl -MTime::HiRes=time -e 'printf "%.3f\n", time')"
DVS_NUM_THREADS="$THREADS" dvs add data/derived/file_*.bin
CLI_END="$(perl -MTime::HiRes=time -e 'printf "%.3f\n", time')"
CLI_ELAPSED="$(perl -e "printf '%.3f', $CLI_END - $CLI_START")"
printf '<<< CLI elapsed: %s s\n\n' "$CLI_ELAPSED"

# ── R: set_dvs_threads() ────────────────────────────────────────────

DVS_REPO_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"

cd "$DVS_REPO_RPKG"
git init .

# Copy the same files so content + hashing work is identical
cp -r "$DVS_REPO_CLI/data" "$DVS_REPO_RPKG/data"

printf '\n>>> R add with set_dvs_threads(%s)\n' "$THREADS"
tee /dev/stderr <<EOF | Rscript -
library(dvs)

dvs_init("$DVS_STORAGE_RPKG")
set_dvs_threads(${THREADS}L)

cat(sprintf("dvs.num_threads option = %s\n", getOption("dvs.num_threads")))

start <- proc.time()
dvs_add(glob = "data/derived/*")
elapsed <- (proc.time() - start)[["elapsed"]]

cat(sprintf("<<< R (set_dvs_threads) elapsed: %.3f s\n", elapsed))
EOF

# ── R: withr::with_options() ────────────────────────────────────────

DVS_REPO_WITHR="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_WITHR="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"

cd "$DVS_REPO_WITHR"
git init .
cp -r "$DVS_REPO_CLI/data" "$DVS_REPO_WITHR/data"

printf '\n>>> R add with withr::with_options(dvs.num_threads = %s)\n' "$THREADS"
tee /dev/stderr <<EOF | Rscript -
library(dvs)

dvs_init("$DVS_STORAGE_WITHR")

cat(sprintf("dvs.num_threads before with_options = %s\n",
            deparse(getOption("dvs.num_threads"))))

start <- proc.time()
withr::with_options(list(dvs.num_threads = ${THREADS}L), {
  dvs_add(glob = "data/derived/*")
})
elapsed <- (proc.time() - start)[["elapsed"]]

cat(sprintf("<<< R (withr) elapsed: %.3f s\n", elapsed))
EOF

# ── Summary ─────────────────────────────────────────────────────────

printf '\n=== Results (%d files × %s, threads=%s) ===\n' \
  "$N_FILES" "$FILE_SIZE" "$THREADS"
printf '  CLI  (DVS_NUM_THREADS=%s): %s s\n' "$THREADS" "$CLI_ELAPSED"
printf '  R timings printed above from proc.time()\n'
printf '=== Done ===\n'
