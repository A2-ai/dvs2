#!/usr/bin/env bash

# Showcase dvs status features: path filtering and recursive flag
# Compares CLI and R package behavior side-by-side.

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

# ── Setup: two repos (CLI + R), nested directory structure ──

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
DVS_STORAGE_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"

DVS_REPO_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
DVS_STORAGE_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"

# ── Init ──

cd "$DVS_REPO_CLI"
dvs init "$DVS_STORAGE_CLI"

cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_RPKG")
EOF

# ── Create files in nested directories ──

cd "$DVS_REPO_CLI"
mkfiles 3 1K data/raw
mkfiles 2 1K data/derived
mkfiles 2 1K models/v1

dvs add --glob "data/**/*.bin"
dvs add --glob "models/**/*.bin"

cd "$DVS_REPO_RPKG"
mkfiles 3 1K data/raw
mkfiles 2 1K data/derived
mkfiles 2 1K models/v1

print_eval_rscript <<EOF
library(dvs)
dvs_add(glob = "data/**/*.bin")
dvs_add(glob = "models/**/*.bin")
EOF

# ── 1. Status: all files (default) ──

echo "=== CLI: dvs status (all files) ==="
cd "$DVS_REPO_CLI"
dvs status

echo "=== R: dvs_status() (all files) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 2. Status: filter to a single file ──

echo "=== CLI: dvs status data/raw/file_1.bin ==="
cd "$DVS_REPO_CLI"
dvs status data/raw/file_1.bin

echo "=== R: dvs_status(files = 'data/raw/file_1.bin') ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = "data/raw/file_1.bin")
EOF

# ── 3. Status: filter to a directory (non-recursive) ──

echo "=== CLI: dvs status data/ (non-recursive — direct children only) ==="
cd "$DVS_REPO_CLI"
dvs status data/

echo "=== R: dvs_status(files = 'data/') (non-recursive) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = "data/")
EOF

# ── 4. Status: filter to a directory (recursive) ──

echo "=== CLI: dvs status -r data/ (recursive — all descendants) ==="
cd "$DVS_REPO_CLI"
dvs status -r data/

echo "=== R: dvs_status(files = 'data/', recursive = TRUE) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = "data/", recursive = TRUE)
EOF

# ── 5. Status: filter with status flags ──

echo "=== CLI: dvs status --absent (show only absent files) ==="
cd "$DVS_REPO_CLI"
dvs status --absent

echo "=== R: dvs_status(status = 'absent') ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(status = "absent")
EOF

# ── 6. Retrieve some files, then show mixed status ──

cd "$DVS_REPO_CLI"
dvs get data/raw/file_1.bin

echo "=== CLI: dvs status (mixed: 1 current, rest absent) ==="
dvs status

cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_get(files = "data/raw/file_1.bin")
EOF

echo "=== R: dvs_status() (mixed: 1 current, rest absent) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 7. Combined: path filter + status flag ──

echo "=== CLI: dvs status -r data/ --current ==="
cd "$DVS_REPO_CLI"
dvs status -r data/ --current

echo "=== R: dvs_status(files = 'data/', recursive = TRUE, status = 'current') ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = "data/", recursive = TRUE, status = "current")
EOF

echo "=== CLI: dvs status -r data/ --absent ==="
cd "$DVS_REPO_CLI"
dvs status -r data/ --absent

echo "=== R: dvs_status(files = 'data/', recursive = TRUE, status = 'absent') ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = "data/", recursive = TRUE, status = "absent")
EOF

# ── 8. Multiple paths ──

echo "=== CLI: dvs status data/raw/file_1.bin models/ ==="
cd "$DVS_REPO_CLI"
dvs status data/raw/file_1.bin models/

echo "=== R: dvs_status(files = c('data/raw/file_1.bin', 'models/')) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(files = c("data/raw/file_1.bin", "models/"))
EOF
