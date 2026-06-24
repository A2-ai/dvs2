#!/usr/bin/env bash

# Showcase how dvs_add / dvs_get / dvs_status surface results under the
# fail-fast (#240) model:
#
#   add / get
#     - all inputs valid          → data.frame, one row per file
#     - any INPUT path invalid     → RAISES, the whole batch is refused and
#       (nonexistent for add,         nothing is written/retrieved
#        untracked for get)
#     - failure AFTER the batch    → does NOT raise; that file is one row with
#       starts (copy permission       its `error` column set and the success
#       error / post-retrieval        columns NA, next to the rows that succeeded
#       hash mismatch)
#
#   status
#     - never raises as a whole. A file whose .dvs metadata is unparseable is
#       reported as a per-file error row, next to the healthy rows.
#
# (The pre-#240 best-effort `list(Success = df, Error = df)` shape no longer
# exists. specs.md: dvs_add L207, dvs_get L266, dvs_status L307-309.)

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

echo "NOTE: \`just install-all\` should have been called prior to this so the installed dvs R package reflects the current branch."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

DVS_REPO_RPKG="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_rpkg_XXX)"
RUN_SUFFIX="${DVS_REPO_RPKG##*_}"
DVS_STORAGE_RPKG="$SCRIPT_DIR/dvs_storage_rpkg_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_RPKG"

cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_RPKG")
EOF

mkfiles 5 1K data
say "--- tree (5 files in data/) ---"
tree --noreport

# show()      prints the shape of a returned value (data.frame row/col/error counts).
# show_call() runs a call and reports EITHER the returned value (via show) OR the
#             raised condition, so a fail-fast raise is displayed, not an abort.
shape_helper='
show <- function(label, x) {
  cat("\n# ", label, "\n", sep = "")
  cat("# class: ", paste(class(x), collapse = "/"), "\n", sep = "")
  if (inherits(x, "data.frame")) {
    cat("# shape: data.frame, ", nrow(x), " row(s), cols: ",
        paste(names(x), collapse = ", "), "\n", sep = "")
    if ("error" %in% names(x)) {
      cat("# error rows: ", sum(!is.na(x[["error"]])), " of ", nrow(x), "\n", sep = "")
    }
    print(x, width = Inf)
  } else {
    print(x)
  }
}
show_call <- function(label, fn) {
  res <- tryCatch(fn(), error = function(e) e)
  if (inherits(res, "condition")) {
    cat("\n# ", label, "\n", sep = "")
    cat("# RAISED: ", class(res)[1], "\n", sep = "")
    cat("# message: ", conditionMessage(res), "\n", sep = "")
  } else {
    show(label, res)
  }
}
'

# ─── 1. dvs_add ────────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_add: variant matrix                                   ="
say "============================================================"

# (a) all inputs valid → data.frame, one row per file
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_add all-valid (5 real files) -> data.frame",
  function() dvs_add(paths = c("data/file_1.bin", "data/file_2.bin",
                               "data/file_3.bin", "data/file_4.bin",
                               "data/file_5.bin")))
EOF

# (b) every input path is nonexistent → RAISES (whole batch refused)
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_add all-invalid (3 nonexistent paths) -> RAISES",
  function() dvs_add(paths = c("data/missing_a.bin",
                               "data/missing_b.bin",
                               "data/missing_c.bin")))
EOF

# (c) one input path nonexistent among valid ones → RAISES (nothing written)
mkrandfile data/mix_x.bin 1K
mkrandfile data/mix_y.bin 1K
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_add valid + one missing input -> RAISES (all-or-nothing)",
  function() dvs_add(paths = c("data/mix_x.bin",
                               "data/mix_y.bin",
                               "data/missing_only.bin")))
EOF

# (d) all inputs EXIST, but one is unreadable (chmod 000): the batch starts, then
#     the copy of the unreadable file fails AFTER the fact -> does NOT raise; it
#     comes back as one error row (success columns NA) next to the success rows.
mkrandfile data/ps_ok1.bin 1K
mkrandfile data/ps_ok2.bin 1K
mkrandfile data/ps_bad.bin 1K
chmod 000 data/ps_bad.bin
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_add post-start failure (1 unreadable) -> data.frame with an error row",
  function() dvs_add(paths = c("data/ps_ok1.bin",
                               "data/ps_ok2.bin",
                               "data/ps_bad.bin")))
EOF
chmod 644 data/ps_bad.bin

# ─── 2. dvs_get ────────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_get: variant matrix                                   ="
say "============================================================"

rm -f data/file_*.bin

# (a) all requested paths tracked → data.frame, one row per file
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_get all-tracked (5 files) -> data.frame",
  function() dvs_get(paths = c("data/file_1.bin", "data/file_2.bin",
                               "data/file_3.bin", "data/file_4.bin",
                               "data/file_5.bin")))
EOF

# (b) every requested path is untracked → RAISES (whole batch refused)
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_get all-untracked (3 paths) -> RAISES",
  function() dvs_get(paths = c("data/never_tracked_a.bin",
                               "data/never_tracked_b.bin",
                               "data/never_tracked_c.bin")))
EOF

rm -f data/file_*.bin

# (c) one requested path untracked among tracked ones → RAISES (nothing retrieved)
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_get tracked + one untracked -> RAISES (all-or-nothing)",
  function() dvs_get(paths = c("data/file_1.bin",
                               "data/file_2.bin",
                               "data/never_tracked.bin")))
EOF

# NOTE: the get analog of the dvs_add post-start error row (case d above) is a
# post-retrieval hash mismatch (a tampered storage blob). It does NOT raise; the
# file comes back as one error row with success columns NA. It is not shown here
# because it needs blob tampering (zstd); see ui/cases_get.sh for that scenario.

# ─── 3. dvs_status ─────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_status: variant matrix (never raises as a whole)      ="
say "============================================================"

# (a) all metadata valid → data.frame, all healthy rows (status reports tracked
#     metadata regardless of whether the local file is present)
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_status all-valid (every .dvs metadata parseable)",
  function() dvs_status())
EOF

# Snapshot the .dvs tree so we can corrupt files freely, then restore.
DVS_META_BACKUP="$SCRIPT_DIR/dvs_fixture_meta_backup_$RUN_SUFFIX"
backup_meta() { rm -rf "$DVS_META_BACKUP"; cp -a "$DVS_REPO_RPKG/.dvs" "$DVS_META_BACKUP"; }
restore_meta() { rm -rf "$DVS_REPO_RPKG/.dvs"; cp -a "$DVS_META_BACKUP" "$DVS_REPO_RPKG/.dvs"; }

backup_meta

# (b) every .dvs metadata file corrupted → data.frame, all error rows (no raise)
find "$DVS_REPO_RPKG/.dvs/data" -name '*.dvs' -type f | while read -r f; do
  printf 'not-json' > "$f"
done
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_status all-corrupted (every .dvs unparseable) -> error rows, no raise",
  function() dvs_status())
EOF
restore_meta

# (c) one .dvs metadata file corrupted → data.frame mixing healthy + error rows
say "--- corrupting one .dvs metadata file ---"
printf 'broken' > "$DVS_REPO_RPKG/.dvs/data/file_1.bin.dvs"
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_status one corrupted, rest healthy -> mixed rows",
  function() dvs_status())
EOF
restore_meta

# (d) all but one corrupted → data.frame, one healthy row among error rows
say "--- corrupting all .dvs metadata files except file_2.bin.dvs ---"
find "$DVS_REPO_RPKG/.dvs/data" -name '*.dvs' -type f | while read -r f; do
  case "$f" in
    */file_2.bin.dvs) : ;;
    *) printf 'broken' > "$f" ;;
  esac
done
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show_call("dvs_status one healthy, rest corrupted -> mixed rows",
  function() dvs_status())
EOF
restore_meta

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
