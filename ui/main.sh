#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/scripts/helpers.sh
source "${SCRIPT_DIR}/scripts/helpers.sh"

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"

cd "$DVS_REPO_CLI"

dvs init "$DVS_STORAGE_CLI"

mkfiles 10 10M data/derived

dvs add data/derived/file_*.bin

mkdatasetfiles 10 10M data/derived chickweight

dvs add data/derived/file_chickweight_*.csv


bash "${SCRIPT_DIR}/cleanup.sh"

