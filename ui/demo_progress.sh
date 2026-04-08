#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/helpers.sh"

N="${1:-3}"
SIZE="${2:-500M}"

echo ""
echo "=== Generating $N x $SIZE files ==="
echo ""

FIXTURES="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
cd "$FIXTURES" && git init -q .
mkfiles "$N" "$SIZE" data

# ── CLI ──

CLI_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
CLI_STORAGE="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_cli_XXX)"
cd "$CLI_REPO" && git init -q .
dvs init "$CLI_STORAGE" 2>/dev/null
cp -r "$FIXTURES/data" "$CLI_REPO/data"

echo "=== CLI ADD ==="
dvs add data/*

FILES=(data/*.bin)
rm -f data/*.bin
echo ""
echo "=== CLI GET ==="
dvs get "${FILES[@]}"

# ── R package (cli progress bar from Rust) ──

R_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
R_STORAGE="$(mktemp -d "$SCRIPT_DIR"/dvs_storage_rpkg_XXX)"
cd "$R_REPO" && git init -q .
cp -r "$FIXTURES/data" "$R_REPO/data"

echo ""
echo "=== RPKG ADD ==="
Rscript --vanilla -e '
library(dvs)
invisible(dvs_init("'"$R_STORAGE"'"))
cb <- dvs:::ProgressBarCallback$new()
files <- list.files("data", full.names = TRUE)
invisible(dvs:::dvs_add_impl(files = files, progress_callback = cb))
'

rm -f data/*.bin

echo ""
echo "=== RPKG GET ==="
Rscript --vanilla -e '
library(dvs)
cb <- dvs:::ProgressBarCallback$new()
meta <- list.files(".dvs/data", full.names = FALSE)
files <- file.path("data", sub("[.]dvs$", "", meta))
invisible(dvs:::dvs_get_impl(files = files, progress_callback = cb))
'

echo ""
echo "=== Done ==="
echo "Cleanup: bash $SCRIPT_DIR/cleanup.sh"
