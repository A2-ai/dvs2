#!/usr/bin/env bash

# Logging consistency test for dvs-cli vs dvs-rpkg.
# Exercises log levels (off/info/debug/warn/error), partial failures,
# multi-thread log draining, and live log-level transitions in one R process.
# This is a *correctness* test — output is read visually, not asserted.
#
# Note: RUST_LOG=dvs=<level> only controls dvs-cli. The dvs-rpkg package
# does NOT consult RUST_LOG; use set_dvs_log_level("…") in R instead.

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

echo "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH and the installed dvs R package both reflect the current branch."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

# ============================================================================
# S1. off (silent default)
#     CLI: no RUST_LOG set; R: no set_dvs_log_level call.
#     Both should produce zero log output — only normal operational stdout.
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S1. off (silent default) — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S1="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S1="${FIXTURES_S1##*_}"
DVS_REPO_CLI_S1="$SCRIPT_DIR/dvs_repo_cli_s1_$RUN_SUFFIX_S1"
DVS_STORAGE_CLI_S1="$SCRIPT_DIR/dvs_storage_cli_s1_$RUN_SUFFIX_S1"
DVS_REPO_R_S1="$SCRIPT_DIR/dvs_repo_rpkg_s1_$RUN_SUFFIX_S1"
DVS_STORAGE_R_S1="$SCRIPT_DIR/dvs_storage_rpkg_s1_$RUN_SUFFIX_S1"
mkdir "$DVS_REPO_CLI_S1" "$DVS_STORAGE_CLI_S1" "$DVS_REPO_R_S1" "$DVS_STORAGE_R_S1"

cd "$DVS_REPO_CLI_S1"
dvs init "$DVS_STORAGE_CLI_S1"
mkfiles 3 1K data/derived

say "--- S1 CLI: dvs add (no RUST_LOG) ---"
dvs add data/derived/file_*.bin

cd "$DVS_REPO_R_S1"
mkfiles 3 1K data/derived

say "--- S1 R: dvs_add (no set_dvs_log_level call — default 'off') ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S1")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
EOF

# ============================================================================
# S2. info — clean add + get
#     CLI: RUST_LOG=dvs=info; R: set_dvs_log_level("info").
#     Note: RUST_LOG has no effect on the R package; the R side must use
#     set_dvs_log_level().
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S2. info — clean add + get — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S2="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S2="${FIXTURES_S2##*_}"
DVS_REPO_CLI_S2="$SCRIPT_DIR/dvs_repo_cli_s2_$RUN_SUFFIX_S2"
DVS_STORAGE_CLI_S2="$SCRIPT_DIR/dvs_storage_cli_s2_$RUN_SUFFIX_S2"
DVS_REPO_R_S2="$SCRIPT_DIR/dvs_repo_rpkg_s2_$RUN_SUFFIX_S2"
DVS_STORAGE_R_S2="$SCRIPT_DIR/dvs_storage_rpkg_s2_$RUN_SUFFIX_S2"
mkdir "$DVS_REPO_CLI_S2" "$DVS_STORAGE_CLI_S2" "$DVS_REPO_R_S2" "$DVS_STORAGE_R_S2"

cd "$DVS_REPO_CLI_S2"
dvs init "$DVS_STORAGE_CLI_S2"
mkfiles 5 1K data/derived

say "--- S2 CLI: dvs add (RUST_LOG=dvs=info) ---"
RUST_LOG=dvs=info dvs add data/derived/file_*.bin

say "--- S2 CLI: dvs get (RUST_LOG=dvs=info) ---"
RUST_LOG=dvs=info dvs get data/derived/file_*.bin

cd "$DVS_REPO_R_S2"
mkfiles 5 1K data/derived

say "--- S2 R: dvs_add + dvs_get (set_dvs_log_level(\"info\")) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S2")
set_dvs_log_level("info")
cat("--- add ---\n")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
cat("--- get ---\n")
dvs_get(glob = "data/derived/*.bin") |> print(width = Inf)
EOF

# Asymmetry probe: confirm RUST_LOG has no effect on the R package.
say "--- S2 R: RUST_LOG=dvs=debug should be ignored by R (no log lines expected) ---"
RUST_LOG=dvs=debug Rscript - <<EOF
library(dvs)
setwd("$DVS_REPO_R_S2")
# No set_dvs_log_level() call → still 'off' default → silence.
dvs_get(glob = "data/derived/*.bin") |> invisible()
cat("[asymmetry probe complete — if the line above this is silent, RUST_LOG was correctly ignored]\n")
EOF

# ============================================================================
# S3. debug — clean add + get
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S3. debug — clean add + get — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S3="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S3="${FIXTURES_S3##*_}"
DVS_REPO_CLI_S3="$SCRIPT_DIR/dvs_repo_cli_s3_$RUN_SUFFIX_S3"
DVS_STORAGE_CLI_S3="$SCRIPT_DIR/dvs_storage_cli_s3_$RUN_SUFFIX_S3"
DVS_REPO_R_S3="$SCRIPT_DIR/dvs_repo_rpkg_s3_$RUN_SUFFIX_S3"
DVS_STORAGE_R_S3="$SCRIPT_DIR/dvs_storage_rpkg_s3_$RUN_SUFFIX_S3"
mkdir "$DVS_REPO_CLI_S3" "$DVS_STORAGE_CLI_S3" "$DVS_REPO_R_S3" "$DVS_STORAGE_R_S3"

cd "$DVS_REPO_CLI_S3"
dvs init "$DVS_STORAGE_CLI_S3"
mkfiles 5 1K data/derived

say "--- S3 CLI: dvs add (RUST_LOG=dvs=debug) ---"
RUST_LOG=dvs=debug dvs add data/derived/file_*.bin

say "--- S3 CLI: dvs get (RUST_LOG=dvs=debug) ---"
RUST_LOG=dvs=debug dvs get data/derived/file_*.bin

cd "$DVS_REPO_R_S3"
mkfiles 5 1K data/derived

say "--- S3 R: dvs_add + dvs_get (set_dvs_log_level(\"debug\")) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S3")
set_dvs_log_level("debug")
cat("--- add ---\n")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
cat("--- get ---\n")
dvs_get(glob = "data/derived/*.bin") |> print(width = Inf)
EOF

# ============================================================================
# S4. warn — partial failure (4 real + 1 bogus)
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S4. warn — partial failure (4 real + 1 bogus) — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S4="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S4="${FIXTURES_S4##*_}"
DVS_REPO_CLI_S4="$SCRIPT_DIR/dvs_repo_cli_s4_$RUN_SUFFIX_S4"
DVS_STORAGE_CLI_S4="$SCRIPT_DIR/dvs_storage_cli_s4_$RUN_SUFFIX_S4"
DVS_REPO_R_S4="$SCRIPT_DIR/dvs_repo_rpkg_s4_$RUN_SUFFIX_S4"
DVS_STORAGE_R_S4="$SCRIPT_DIR/dvs_storage_rpkg_s4_$RUN_SUFFIX_S4"
mkdir "$DVS_REPO_CLI_S4" "$DVS_STORAGE_CLI_S4" "$DVS_REPO_R_S4" "$DVS_STORAGE_R_S4"

cd "$DVS_REPO_CLI_S4"
dvs init "$DVS_STORAGE_CLI_S4"
mkfiles 4 1K data/derived

say "--- S4 CLI: dvs add 4 real + 1 bogus (RUST_LOG=dvs=warn) ---"
# || true: the CLI exits nonzero because one path fails; we want to see the
# warn log line and the error row in output, not abort the entire test script.
RUST_LOG=dvs=warn dvs add \
  data/derived/file_1.bin \
  data/derived/file_2.bin \
  data/derived/file_3.bin \
  data/derived/file_4.bin \
  data/derived/does_not_exist.bin || true

cd "$DVS_REPO_R_S4"
mkfiles 4 1K data/derived

say "--- S4 R: dvs_add 4 real + 1 bogus (set_dvs_log_level(\"warn\")) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S4")
set_dvs_log_level("warn")
# Wrap in tryCatch: the R wrapper raises an error if any file fails. We want
# the script to continue regardless so subsequent scenarios still run.
tryCatch(
  dvs_add(paths = c(
    "data/derived/file_1.bin",
    "data/derived/file_2.bin",
    "data/derived/file_3.bin",
    "data/derived/file_4.bin",
    "data/derived/does_not_exist.bin"
  )) |> print(width = Inf),
  error = function(e) {
    cat("Caught R error (expected):", conditionMessage(e), "\n")
  }
)
EOF

# ============================================================================
# S5. error — entirely bad path
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S5. error — entirely bad path — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S5="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S5="${FIXTURES_S5##*_}"
DVS_REPO_CLI_S5="$SCRIPT_DIR/dvs_repo_cli_s5_$RUN_SUFFIX_S5"
DVS_STORAGE_CLI_S5="$SCRIPT_DIR/dvs_storage_cli_s5_$RUN_SUFFIX_S5"
DVS_REPO_R_S5="$SCRIPT_DIR/dvs_repo_rpkg_s5_$RUN_SUFFIX_S5"
DVS_STORAGE_R_S5="$SCRIPT_DIR/dvs_storage_rpkg_s5_$RUN_SUFFIX_S5"
mkdir "$DVS_REPO_CLI_S5" "$DVS_STORAGE_CLI_S5" "$DVS_REPO_R_S5" "$DVS_STORAGE_R_S5"

cd "$DVS_REPO_CLI_S5"
dvs init "$DVS_STORAGE_CLI_S5"

say "--- S5 CLI: dvs add /no/such/path/file.bin (expect 'Error:' line, nonzero exit) ---"
# || true: the path doesn't exist so dvs exits nonzero; we capture the error
# output for human inspection and intentionally continue.
RUST_LOG=dvs=error dvs add /no/such/path/file.bin || true

cd "$DVS_REPO_R_S5"

say "--- S5 R: dvs_add bad path (expect R error message, no panic) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S5")
set_dvs_log_level("error")
tryCatch(
  dvs_add(paths = "/no/such/path/file.bin"),
  error = function(e) {
    cat("Caught R error (expected):", conditionMessage(e), "\n")
  }
)
EOF

# ============================================================================
# S6. threads × debug — 30 files, 4 threads
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S6. threads × debug — 30 files, 4 threads — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S6="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S6="${FIXTURES_S6##*_}"
DVS_REPO_CLI_S6="$SCRIPT_DIR/dvs_repo_cli_s6_$RUN_SUFFIX_S6"
DVS_STORAGE_CLI_S6="$SCRIPT_DIR/dvs_storage_cli_s6_$RUN_SUFFIX_S6"
DVS_REPO_R_S6="$SCRIPT_DIR/dvs_repo_rpkg_s6_$RUN_SUFFIX_S6"
DVS_STORAGE_R_S6="$SCRIPT_DIR/dvs_storage_rpkg_s6_$RUN_SUFFIX_S6"
mkdir "$DVS_REPO_CLI_S6" "$DVS_STORAGE_CLI_S6" "$DVS_REPO_R_S6" "$DVS_STORAGE_R_S6"

cd "$DVS_REPO_CLI_S6"
dvs init "$DVS_STORAGE_CLI_S6"
mkfiles 30 1M data/derived

say "--- S6 CLI: dvs add 30×1M, 4 threads (RUST_LOG=dvs=debug) ---"
DVS_NUM_THREADS=4 RUST_LOG=dvs=debug dvs add data/derived/file_*.bin

say "--- S6 CLI: dvs get 30×1M, 4 threads (RUST_LOG=dvs=debug) ---"
DVS_NUM_THREADS=4 RUST_LOG=dvs=debug dvs get data/derived/file_*.bin

cd "$DVS_REPO_R_S6"
mkfiles 30 1M data/derived

say "--- S6 R: dvs_add + dvs_get 30×1M, 4 threads (set_dvs_log_level(\"debug\")) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S6")
set_dvs_threads(4L)
set_dvs_log_level("debug")
cat("--- add ---\n")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
cat("--- get ---\n")
dvs_get(glob = "data/derived/*.bin") |> print(width = Inf)
EOF

# ============================================================================
# S7. threads × warn — partial failure, 4 threads
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S7. threads × warn — partial failure, 4 threads — CLI + R"
say "════════════════════════════════════════════════════════════"

FIXTURES_S7="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S7="${FIXTURES_S7##*_}"
DVS_REPO_CLI_S7="$SCRIPT_DIR/dvs_repo_cli_s7_$RUN_SUFFIX_S7"
DVS_STORAGE_CLI_S7="$SCRIPT_DIR/dvs_storage_cli_s7_$RUN_SUFFIX_S7"
DVS_REPO_R_S7="$SCRIPT_DIR/dvs_repo_rpkg_s7_$RUN_SUFFIX_S7"
DVS_STORAGE_R_S7="$SCRIPT_DIR/dvs_storage_rpkg_s7_$RUN_SUFFIX_S7"
mkdir "$DVS_REPO_CLI_S7" "$DVS_STORAGE_CLI_S7" "$DVS_REPO_R_S7" "$DVS_STORAGE_R_S7"

cd "$DVS_REPO_CLI_S7"
dvs init "$DVS_STORAGE_CLI_S7"
mkfiles 10 1M data/derived

say "--- S7 CLI: dvs add 10 real + 1 bogus, 4 threads (RUST_LOG=dvs=warn) ---"
# || true: same reason as S4 — one bad path causes a nonzero exit; we want the
# warn log line for inspection without aborting the script.
DVS_NUM_THREADS=4 RUST_LOG=dvs=warn dvs add \
  data/derived/file_01.bin \
  data/derived/file_02.bin \
  data/derived/file_03.bin \
  data/derived/file_04.bin \
  data/derived/file_05.bin \
  data/derived/file_06.bin \
  data/derived/file_07.bin \
  data/derived/file_08.bin \
  data/derived/file_09.bin \
  data/derived/file_10.bin \
  data/derived/bogus_thread.bin || true

cd "$DVS_REPO_R_S7"
mkfiles 10 1M data/derived

say "--- S7 R: dvs_add 10 real + 1 bogus, 4 threads (set_dvs_log_level(\"warn\")) ---"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_R_S7")
set_dvs_threads(4L)
set_dvs_log_level("warn")
tryCatch(
  dvs_add(paths = c(
    "data/derived/file_01.bin",
    "data/derived/file_02.bin",
    "data/derived/file_03.bin",
    "data/derived/file_04.bin",
    "data/derived/file_05.bin",
    "data/derived/file_06.bin",
    "data/derived/file_07.bin",
    "data/derived/file_08.bin",
    "data/derived/file_09.bin",
    "data/derived/file_10.bin",
    "data/derived/bogus_thread.bin"
  )) |> print(width = Inf),
  error = function(e) {
    cat("Caught R error (expected):", conditionMessage(e), "\n")
  }
)
EOF

# ============================================================================
# S8. R-only — live log-level transitions in one process
#     debug → off → warn across three add/get operations.
# ============================================================================

say
say "════════════════════════════════════════════════════════════"
say "  S8. R-only — live log-level transitions: debug → off → warn"
say "════════════════════════════════════════════════════════════"

FIXTURES_S8="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX_S8="${FIXTURES_S8##*_}"
DVS_REPO_R_S8A="$SCRIPT_DIR/dvs_repo_rpkg_s8a_$RUN_SUFFIX_S8"
DVS_STORAGE_R_S8A="$SCRIPT_DIR/dvs_storage_rpkg_s8a_$RUN_SUFFIX_S8"
DVS_REPO_R_S8B="$SCRIPT_DIR/dvs_repo_rpkg_s8b_$RUN_SUFFIX_S8"
DVS_STORAGE_R_S8B="$SCRIPT_DIR/dvs_storage_rpkg_s8b_$RUN_SUFFIX_S8"
DVS_REPO_R_S8C="$SCRIPT_DIR/dvs_repo_rpkg_s8c_$RUN_SUFFIX_S8"
DVS_STORAGE_R_S8C="$SCRIPT_DIR/dvs_storage_rpkg_s8c_$RUN_SUFFIX_S8"
mkdir "$DVS_REPO_R_S8A" "$DVS_STORAGE_R_S8A" \
      "$DVS_REPO_R_S8B" "$DVS_STORAGE_R_S8B" \
      "$DVS_REPO_R_S8C" "$DVS_STORAGE_R_S8C"

cd "$DVS_REPO_R_S8A" && mkfiles 3 1K data/derived
cd "$DVS_REPO_R_S8B" && mkfiles 3 1K data/derived
cd "$DVS_REPO_R_S8C" && mkfiles 3 1K data/derived

say "--- S8 R: debug → off → warn transitions in one Rscript process ---"
print_eval_rscript <<EOF
library(dvs)

# ── Phase A: debug — verbose output expected ──
setwd("$DVS_REPO_R_S8A")
dvs_init("$DVS_STORAGE_R_S8A")
set_dvs_log_level("debug")
cat("\n[S8 phase A: debug]\n")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
dvs_get(glob = "data/derived/*.bin") |> print(width = Inf)

# ── Phase B: off — silent; only result frame printed ──
setwd("$DVS_REPO_R_S8B")
dvs_init("$DVS_STORAGE_R_S8B")
set_dvs_log_level("off")
cat("\n[S8 phase B: off — expect no log lines below]\n")
dvs_add(glob = "data/derived/*.bin") |> print(width = Inf)
dvs_get(glob = "data/derived/*.bin") |> print(width = Inf)

# ── Phase C: warn — only warnings; a bogus file triggers one warn line ──
setwd("$DVS_REPO_R_S8C")
dvs_init("$DVS_STORAGE_R_S8C")
set_dvs_log_level("warn")
cat("\n[S8 phase C: warn — expect one 'Failed to add' line]\n")
tryCatch(
  dvs_add(paths = c(
    "data/derived/file_1.bin",
    "data/derived/file_2.bin",
    "data/derived/file_3.bin",
    "data/derived/phase_c_bogus.bin"
  )) |> print(width = Inf),
  error = function(e) {
    cat("Caught R error (expected):", conditionMessage(e), "\n")
  }
)
EOF

say
say "════════════════════════════════════════════════════════════"
say "  DONE — all 8 scenarios complete"
say "════════════════════════════════════════════════════════════"

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
