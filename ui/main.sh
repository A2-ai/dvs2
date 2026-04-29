#!/usr/bin/env bash

# note: the -x shows the script command in output
set -euox pipefail
# prints the line in script that errors
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI"

# region: INIT

cd "$DVS_REPO_CLI"

dvs init "$DVS_STORAGE_CLI"

ls -a "$DVS_REPO_CLI" "$DVS_STORAGE_CLI"

DVS_REPO_RPKG="$SCRIPT_DIR/dvs_repo_rpkg_$RUN_SUFFIX"
DVS_STORAGE_RPKG="$SCRIPT_DIR/dvs_storage_rpkg_$RUN_SUFFIX"
mkdir "$DVS_REPO_RPKG" "$DVS_STORAGE_RPKG"

cd "$DVS_REPO_RPKG"

# this `tee` prints the R-script being executed
tee /dev/stderr <<EOF | Rscript -
library(dvs)

dvs_init("$DVS_STORAGE_RPKG")
EOF

# region: ADD

cd "$DVS_REPO_CLI"
mkfiles 5 10M data/derived
dvs add data/derived/file_*.bin

mkdatasetfiles 5 10M data/derived chickweight
dvs add data/derived/file_chickweight_*.csv

cd "$DVS_REPO_RPKG"

mkfiles 5 10M data/derived
mkdatasetfiles 5 10M data/derived chickweight

tee /dev/stderr <<EOF | Rscript -
library(dvs)

# dvs_add("data/derived") # ERROR

dvs_add("$DVS_REPO_RPKG/data/derived", glob = "*") # WORKS

# conclusion: the data-frame does not contain the absolute paths even if we give it absolute paths of the files
# data_derived_files <- c($(find "$DVS_REPO_RPKG"/data/derived -type f | sed 's/.*/"&"/' | paste -sd, -))
# dvs_add(data_derived_files) # WORKS
EOF

# region: STATUS

cd "$DVS_REPO_CLI"

dvs status

# NOT IMPLEMENTED:
# dvs status data/derived/file_chickweight_*

cd "$DVS_REPO_RPKG"

print_eval_rscript <<EOF
library(dvs)

dvs_status()

EOF

# TODO:
#   [ ] make tibble a Suggests, and _impl post-fix the dvs_* from Rust stuff
#   [ ] truncate the hash

# # Compare dvs.toml (created by init)
# diff "${DVS_REPO_CLI}"/dvs.toml "${DVS_REPO_RPKG}"/dvs.toml

# # Compare .dvs metadata directories
# diff -rN "${DVS_REPO_CLI}"/.dvs "${DVS_REPO_RPKG}"/.dvs

# # Compare storage directories
# diff -rN "${DVS_STORAGE_CLI}" "${DVS_STORAGE_RPKG}"




printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
