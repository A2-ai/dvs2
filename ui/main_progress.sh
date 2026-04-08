#!/usr/bin/env bash

# Progress bar comparison test.
# Tests how progress reporting looks in dvs-cli vs dvs-rpkg for:
#   1. 100 x 1MB files  (many small files)
#   2. 1 x 500MB file   (single large file)

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

source "${SCRIPT_DIR}/helpers.sh"

# Helper: forced R progress callback (negative=file size, positive=bytes)
R_PROGRESS_CB='
.total <- 0; .current <- 0; .pb <- NULL
cb <- dvs:::ProgressBarCallback$new(function(n) {
  if (n < 0) { .total <<- .total + abs(n) }
  else { .current <<- .current + n }
  if (.total > 0) {
    ratio <- min(.current / .total, 1)
    if (is.null(.pb)) {
      .pb <<- progress::progress_bar$new(
        format = "  [:bar] :percent eta: :eta",
        total = 1e6, clear = FALSE, force = TRUE
      )
    }
    .pb$update(ratio)
  }
})
'

# ── Scenario 1: 100 x 1MB files ────────────────────────────────────

printf '\n\n========================================\n'
printf '  SCENARIO 1: 100 x 1MB files\n'
printf '========================================\n\n'

FIXTURES_1="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
cd "$FIXTURES_1" && git init .
mkfiles 100 1M data/derived

printf '\n--- CLI ADD: 100 x 1MB ---\n'
DVS_REPO_CLI_1="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI_1="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"
cd "$DVS_REPO_CLI_1" && git init .
dvs init "$DVS_STORAGE_CLI_1"
cp -r "$FIXTURES_1/data" "$DVS_REPO_CLI_1/data"
time dvs add data/derived/*

printf '\n--- CLI GET: 100 x 1MB ---\n'
time dvs get data/derived/*

printf '\n--- RPKG ADD: 100 x 1MB ---\n'
DVS_REPO_R_1="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R_1="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
cd "$DVS_REPO_R_1" && git init .
cp -r "$FIXTURES_1/data" "$DVS_REPO_R_1/data"

tee /dev/stderr <<EOF | Rscript -
library(dvs)
dvs_init("$DVS_STORAGE_R_1")
${R_PROGRESS_CB}
files <- list.files("data/derived", pattern = "\\\\.bin$", full.names = TRUE)
cat(sprintf("\n>>> dvs_add (%d x 1MB):\n", length(files)))
system.time(dvs:::dvs_add_impl(files = files, progress_callback = cb))
EOF

printf '\n--- RPKG GET: 100 x 1MB ---\n'
cd "$DVS_REPO_R_1"

tee /dev/stderr <<EOF | Rscript -
library(dvs)
${R_PROGRESS_CB}
meta_files <- list.files(".dvs/data/derived", pattern = "\\\\.bin\\\\.dvs$", full.names = FALSE)
files <- file.path("data/derived", sub("\\\\.dvs$", "", meta_files))
cat(sprintf("\n>>> dvs_get (%d x 1MB):\n", length(files)))
system.time(dvs:::dvs_get_impl(files = files, progress_callback = cb))
EOF

# ── Scenario 2: 1 x 500MB file ─────────────────────────────────────

printf '\n\n========================================\n'
printf '  SCENARIO 2: 1 x 500MB file\n'
printf '========================================\n\n'

FIXTURES_2="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
cd "$FIXTURES_2" && git init .
mkfiles 1 500M data/derived

printf '\n--- CLI ADD: 1 x 500MB ---\n'
DVS_REPO_CLI_2="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI_2="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"
cd "$DVS_REPO_CLI_2" && git init .
dvs init "$DVS_STORAGE_CLI_2"
cp -r "$FIXTURES_2/data" "$DVS_REPO_CLI_2/data"
time dvs add data/derived/file_1.bin

printf '\n--- CLI GET: 1 x 500MB ---\n'
time dvs get data/derived/file_1.bin

printf '\n--- RPKG ADD: 1 x 500MB ---\n'
DVS_REPO_R_2="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_R_2="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
cd "$DVS_REPO_R_2" && git init .
cp -r "$FIXTURES_2/data" "$DVS_REPO_R_2/data"

tee /dev/stderr <<EOF | Rscript -
library(dvs)
dvs_init("$DVS_STORAGE_R_2")
${R_PROGRESS_CB}
cat("\n>>> dvs_add (1 x 500MB):\n")
system.time(dvs:::dvs_add_impl(files = "data/derived/file_1.bin", progress_callback = cb))
EOF

printf '\n--- RPKG GET: 1 x 500MB ---\n'
cd "$DVS_REPO_R_2"

tee /dev/stderr <<EOF | Rscript -
library(dvs)
${R_PROGRESS_CB}
cat("\n>>> dvs_get (1 x 500MB):\n")
system.time(dvs:::dvs_get_impl(files = "data/derived/file_1.bin", progress_callback = cb))
EOF

printf '\n\n========================================\n  DONE\n========================================\n'
printf 'Cleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
