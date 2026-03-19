#!/usr/bin/env bash

set -euox pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/scripts/helpers.sh
source "${SCRIPT_DIR}/scripts/helpers.sh"

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"

cd "$DVS_REPO_CLI"

dvs init "$DVS_STORAGE_CLI"

mkfiles 1 10M data/derived

dvs add data/derived/file_*.bin

mkdatasetfiles 1 10M data/derived chickweight

dvs add data/derived/file_chickweight_*.csv

ls -a "$DVS_REPO_CLI"/.dvs "$DVS_STORAGE_CLI"

DVS_REPO_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"

cd $DVS_REPO_RPKG

Rscript - <<EOF
# cat("Storage is at: $DVS_STORAGE_CLI\n")
library(dvs)

dvs_init("$DVS_STORAGE_RPKG")
EOF


# # dvs init ${DVS_STORAGE_RPKG}

# cd "$DVS_REPO_CLI"

# Rscript - <<EOF
# # cat("Storage is at: $DVS_STORAGE_RPKG\n")
# library(dvs)

# dvs_init("$DVS_STORAGE_RPKG")
# EOF


diff -r "${DVS_REPO_CLI}"/.dvs "${DVS_REPO_RPKG}"/.dvs


# This will delete everything.
# bash ${SCRIPT_DIR}/cleanup.sh
