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

say "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH and the installed dvs R package both reflect the current branch."

# ── Scenario 1: 100 x 1MB files ────────────────────────────────────

printf '\n\n========================================\n'
printf '  SCENARIO 1: 100 x 1MB files\n'
printf '========================================\n\n'

# All scenario-1 dirs share one mktemp suffix so it's obvious they belong
# together: dvs_fixture_AbC, dvs_repo_cli_AbC, dvs_storage_cli_AbC, etc.
FIXTURES_1="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_1="${FIXTURES_1##*_}"
DVS_REPO_CLI_1="$SCRIPT_DIR/dvs_repo_cli_$RUN_SUFFIX_1"
DVS_STORAGE_CLI_1="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX_1"
DVS_REPO_R_1="$SCRIPT_DIR/dvs_repo_rpkg_$RUN_SUFFIX_1"
DVS_STORAGE_R_1="$SCRIPT_DIR/dvs_storage_rpkg_$RUN_SUFFIX_1"
mkdir "$DVS_REPO_CLI_1" "$DVS_STORAGE_CLI_1" "$DVS_REPO_R_1" "$DVS_STORAGE_R_1"
cd "$FIXTURES_1"
mkfiles 100 1M data/derived

printf '\n--- CLI ADD: 100 x 1MB ---\n'
cd "$DVS_REPO_CLI_1"
dvs init "$DVS_STORAGE_CLI_1"
cp -r "$FIXTURES_1/data" "$DVS_REPO_CLI_1/data"
time dvs add data/derived/*

printf '\n--- CLI GET: 100 x 1MB ---\n'
time dvs get data/derived/*

printf '\n--- RPKG ADD: 100 x 1MB ---\n'
cd "$DVS_REPO_R_1"
cp -r "$FIXTURES_1/data" "$DVS_REPO_R_1/data"

Rscript --vanilla -e '
library(dvs)
dvs_init("'"$DVS_STORAGE_R_1"'")
cat("\n>>> dvs_add (100 x 1MB):\n")
system.time(dvs_add(glob = "data/derived/*.bin"))
'

printf '\n--- RPKG GET: 100 x 1MB ---\n'
cd "$DVS_REPO_R_1"

Rscript --vanilla -e '
library(dvs)
cat("\n>>> dvs_get (100 x 1MB):\n")
system.time(dvs_get(glob = "data/derived/*.bin"))
'

# ── Scenario 2: 1 x 500MB file ─────────────────────────────────────

printf '\n\n========================================\n'
printf '  SCENARIO 2: 1 x 500MB file\n'
printf '========================================\n\n'

# Scenario-2 dirs share their own suffix, distinct from scenario-1's.
FIXTURES_2="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_2="${FIXTURES_2##*_}"
DVS_REPO_CLI_2="$SCRIPT_DIR/dvs_repo_cli_$RUN_SUFFIX_2"
DVS_STORAGE_CLI_2="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX_2"
DVS_REPO_R_2="$SCRIPT_DIR/dvs_repo_rpkg_$RUN_SUFFIX_2"
DVS_STORAGE_R_2="$SCRIPT_DIR/dvs_storage_rpkg_$RUN_SUFFIX_2"
mkdir "$DVS_REPO_CLI_2" "$DVS_STORAGE_CLI_2" "$DVS_REPO_R_2" "$DVS_STORAGE_R_2"
cd "$FIXTURES_2"
mkfiles 1 500M data/derived

printf '\n--- CLI ADD: 1 x 500MB ---\n'
cd "$DVS_REPO_CLI_2"
dvs init "$DVS_STORAGE_CLI_2"
cp -r "$FIXTURES_2/data" "$DVS_REPO_CLI_2/data"
time dvs add data/derived/file_1.bin

printf '\n--- CLI GET: 1 x 500MB ---\n'
time dvs get data/derived/file_1.bin

printf '\n--- RPKG ADD: 1 x 500MB ---\n'
cd "$DVS_REPO_R_2"
cp -r "$FIXTURES_2/data" "$DVS_REPO_R_2/data"

Rscript --vanilla -e '
library(dvs)
dvs_init("'"$DVS_STORAGE_R_2"'")
cat("\n>>> dvs_add (1 x 500MB):\n")
system.time(dvs_add(paths = "data/derived/file_1.bin"))
'

printf '\n--- RPKG GET: 1 x 500MB ---\n'
cd "$DVS_REPO_R_2"

Rscript --vanilla -e '
library(dvs)
cat("\n>>> dvs_get (1 x 500MB):\n")
system.time(dvs_get(paths = "data/derived/file_1.bin"))
'

printf '\n\n========================================\n  DONE\n========================================\n'
printf 'Cleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
