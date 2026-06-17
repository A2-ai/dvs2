#!/usr/bin/env bash

# Showcase dvs init across its option matrix.
# Each scenario uses a fresh repo + storage dir pair so we can compare the
# resulting `dvs.toml` (CLI) and `dvs_init` return tibble (R) side-by-side.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH and the installed dvs R package both reflect the current branch."

# Pick a unix group the current user actually belongs to. macOS default is
# `staff`, linux distros vary; fall back to the user's primary group name.
USER_GROUP="$(id -gn)"

# All dirs in this run share one mktemp suffix so it's obvious they belong
# together. Per scenario dirs use an _S<n>_ infix to stay distinct while
# keeping the same trailing run-suffix.
STAGE_DIR="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX="${STAGE_DIR##*_}"

scenario_dirs() {
  local label="${1:?label is required}"
  REPO_CLI="$SCRIPT_DIR/dvs_repo_cli_${label}_$RUN_SUFFIX"
  STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_${label}_$RUN_SUFFIX"
  REPO_R="$SCRIPT_DIR/dvs_repo_rpkg_${label}_$RUN_SUFFIX"
  STORAGE_R="$SCRIPT_DIR/dvs_storage_rpkg_${label}_$RUN_SUFFIX"
  mkdir "$REPO_CLI" "$STORAGE_CLI" "$REPO_R" "$STORAGE_R"
}

# ── 1. Default: storage_path only (zstd, default .dvs metadata folder) ──

say
say "============================================================"
say "  1. Default: storage_path only"
say "============================================================"

scenario_dirs S1
cd "$REPO_CLI"
dvs init "$STORAGE_CLI"

say "--- CLI tree after init ---"
tree --noreport -a -I '.git'
say "--- CLI dvs.toml ---"
cat dvs.toml

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
result <- dvs_init(storage_path = "$STORAGE_R")
print(result, width = Inf)
cat("\n--- str() ---\n")
str(result)
EOF

say "--- R tree after init ---"
tree --noreport -a -I '.git' "$REPO_R"
say "--- R dvs.toml ---"
cat "$REPO_R/dvs.toml"

# ── 2. compression = "none" / --no-compression ──

say
say "============================================================"
say "  2. compression = 'none' / --no-compression"
say "============================================================"

scenario_dirs S2
cd "$REPO_CLI"
dvs init --no-compression "$STORAGE_CLI"

say "--- CLI dvs.toml ---"
cat dvs.toml

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
result <- dvs_init(storage_path = "$STORAGE_R", compression = "none")
print(result, width = Inf)
EOF

say "--- R dvs.toml ---"
cat "$REPO_R/dvs.toml"

# ── 3. Custom metadata_folder_name ──

say
say "============================================================"
say "  3. metadata_folder_name = 'custom_meta'"
say "============================================================"

scenario_dirs S3
cd "$REPO_CLI"
dvs init --metadata-folder-name custom_meta "$STORAGE_CLI"

say "--- CLI tree (expect custom_meta/, not .dvs/) ---"
tree --noreport -a -I '.git'
say "--- CLI dvs.toml ---"
cat dvs.toml

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
result <- dvs_init(
  storage_path = "$STORAGE_R",
  metadata_folder_name = "custom_meta"
)
print(result, width = Inf)
EOF

say "--- R tree (expect custom_meta/, not .dvs/) ---"
tree --noreport -a -I '.git' "$REPO_R"
say "--- R dvs.toml ---"
cat "$REPO_R/dvs.toml"

# ── 4. group = current user's primary group ──

say
say "============================================================"
say "  4. group = '$USER_GROUP'"
say "============================================================"

scenario_dirs S4
cd "$REPO_CLI"
dvs init --group "$USER_GROUP" "$STORAGE_CLI"

say "--- CLI dvs.toml (expect group line) ---"
cat dvs.toml

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
result <- dvs_init(storage_path = "$STORAGE_R", group = "$USER_GROUP")
print(result, width = Inf)
EOF

say "--- R dvs.toml (expect group line) ---"
cat "$REPO_R/dvs.toml"

# ── 5. All options combined ──

say
say "============================================================"
say "  5. All options: --no-compression + custom_meta + group"
say "============================================================"

scenario_dirs S5
cd "$REPO_CLI"
dvs init \
  --no-compression \
  --metadata-folder-name custom_meta \
  --group "$USER_GROUP" \
  "$STORAGE_CLI"

say "--- CLI dvs.toml ---"
cat dvs.toml

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
result <- dvs_init(
  storage_path        = "$STORAGE_R",
  compression         = "none",
  metadata_folder_name = "custom_meta",
  group               = "$USER_GROUP"
)
print(result, width = Inf)
EOF

say "--- R dvs.toml ---"
cat "$REPO_R/dvs.toml"

# ── 6. Failure: storage_path inside the repo (CLI guard mirror) ──

say
say "============================================================"
say "  6. Failure: storage path inside the repo (expect error)"
say "============================================================"

scenario_dirs S6
cd "$REPO_CLI"
# Try to set storage to a subdir of the repo — must refuse.
dvs init "$REPO_CLI/inside_storage" 2>&1 || say "(expected: CLI rejected)"

cd "$REPO_R"
print_eval_rscript <<EOF
library(dvs)
tryCatch(
  dvs_init(storage_path = file.path("$REPO_R", "inside_storage")),
  error = function(e) message("(expected: ", conditionMessage(e), ")")
)
EOF

# ── Done ──

say
say "============================================================"
say "  DONE"
say "============================================================"
printf 'Cleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
