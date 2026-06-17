#!/usr/bin/env bash

# Showcase how dvs_add / dvs_get / dvs_status surface per-variant results:
#   - all-success           → single data.frame
#   - all-failure           → single data.frame
#   - mixed (Success+Error) → named list(Success = df, Error = df)
#
# Built on the new vec_to_dataframe_split helper landed in miniextendr.
#
# Triggers used per command:
#   add:    Error  = path that does not exist (AddDetail::Error "file not found")
#   get:    Error  = path the user asked for but that isn't tracked
#                    (GetDetail::Error "not tracked by DVS")
#   status: Error  = tracked path whose .dvs metadata file is unparseable JSON

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

shape_helper='
show <- function(label, x) {
  cat("\n# ", label, "\n", sep = "")
  cat("# class:    ", paste(class(x), collapse = "/"), "\n", sep = "")
  if (inherits(x, "data.frame")) {
    cat("# shape:    bare data.frame (single-variant)\n", sep = "")
    print(x, width = Inf)
  } else if (is.list(x)) {
    nm <- names(x)
    if (length(x) == 0L) {
      cat("# shape:    empty list (no rows)\n", sep = "")
    } else if (is.null(nm) || all(nm == "")) {
      cat("# shape:    unnamed list of length ", length(x), "\n", sep = "")
    } else {
      cat("# shape:    named list of data.frames, variants: ", paste(nm, collapse = ", "), "\n", sep = "")
    }
    for (n in nm) {
      cat("\n## $", n, "\n", sep = "")
      print(x[[n]], width = Inf)
    }
  } else {
    print(x)
  }
}
'

# ─── 1. dvs_add ────────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_add: variant matrix                                   ="
say "============================================================"

# (a) all-success
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_add all-success (5 real files)",
     dvs_add(paths = c("data/file_1.bin", "data/file_2.bin",
                       "data/file_3.bin", "data/file_4.bin",
                       "data/file_5.bin")))
EOF

# (b) all-failure
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_add all-failure (3 nonexistent paths)",
     dvs_add(paths = c("data/missing_a.bin",
                       "data/missing_b.bin",
                       "data/missing_c.bin")))
EOF

# (c) mixed-success: 2 real + 1 missing
mkrandfile data/mix_x.bin 1K
mkrandfile data/mix_y.bin 1K
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_add mixed-success (2 real + 1 missing)",
     dvs_add(paths = c("data/mix_x.bin",
                       "data/mix_y.bin",
                       "data/missing_only.bin")))
EOF

# (d) mixed-failure: 1 real + 3 missing
mkrandfile data/single_real.bin 1K
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_add mixed-failure (1 real + 3 missing)",
     dvs_add(paths = c("data/single_real.bin",
                       "data/missing_p.bin",
                       "data/missing_q.bin",
                       "data/missing_r.bin")))
EOF

# ─── 2. dvs_get ────────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_get: variant matrix                                   ="
say "============================================================"

rm -f data/file_*.bin

# (a) all-success
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_get all-success (5 tracked files)",
     dvs_get(paths = c("data/file_1.bin", "data/file_2.bin",
                       "data/file_3.bin", "data/file_4.bin",
                       "data/file_5.bin")))
EOF

# (b) all-failure: untracked paths
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_get all-failure (3 untracked paths)",
     dvs_get(paths = c("data/never_tracked_a.bin",
                       "data/never_tracked_b.bin",
                       "data/never_tracked_c.bin")))
EOF

rm -f data/file_*.bin

# (c) mixed-success: 2 tracked + 1 untracked
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_get mixed-success (2 tracked + 1 untracked)",
     dvs_get(paths = c("data/file_1.bin",
                       "data/file_2.bin",
                       "data/never_tracked.bin")))
EOF

rm -f data/file_*.bin

# (d) mixed-failure: 1 tracked + 3 untracked
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_get mixed-failure (1 tracked + 3 untracked)",
     dvs_get(paths = c("data/file_3.bin",
                       "data/missing_get_a.bin",
                       "data/missing_get_b.bin",
                       "data/missing_get_c.bin")))
EOF

# ─── 3. dvs_status ─────────────────────────────────────────────────────────────

say
say "============================================================"
say "= dvs_status: variant matrix                                ="
say "============================================================"

# (a) all-success
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_status all-success (all metadata files valid)",
     dvs_status())
EOF

# Helpers: snapshot the .dvs tree, then we corrupt files freely.
DVS_META_BACKUP="$SCRIPT_DIR/dvs_fixture_meta_backup_$RUN_SUFFIX"
backup_meta() { rm -rf "$DVS_META_BACKUP"; cp -a "$DVS_REPO_RPKG/.dvs" "$DVS_META_BACKUP"; }
restore_meta() { rm -rf "$DVS_REPO_RPKG/.dvs"; cp -a "$DVS_META_BACKUP" "$DVS_REPO_RPKG/.dvs"; }

backup_meta

# (b) all-failure: corrupt every .dvs metadata file
find "$DVS_REPO_RPKG/.dvs/data" -name '*.dvs' -type f | while read -r f; do
  printf 'not-json' > "$f"
done
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_status all-failure (every .dvs metadata corrupted)",
     dvs_status())
EOF
restore_meta

# (c) mixed-success: corrupt one
say "--- corrupting one .dvs metadata file ---"
printf 'broken' > "$DVS_REPO_RPKG/.dvs/data/file_1.bin.dvs"
print_eval_rscript <<EOF
library(dvs)
$shape_helper
show("dvs_status mixed-success (1 corrupted, rest healthy)",
     dvs_status())
EOF
restore_meta

# (d) mixed-failure: corrupt all but one
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
show("dvs_status mixed-failure (1 healthy, rest corrupted)",
     dvs_status())
EOF
restore_meta

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
