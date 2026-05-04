#!/usr/bin/env bash

# Demonstrate recursive vs. non-recursive behaviour across dvs add, dvs get,
# and dvs status — both CLI and R package — over a nested file tree.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH and the installed dvs R package both reflect the current branch."

# ── Setup: two repos (CLI + R), each with its own storage ──
# All four dirs share one mktemp-generated suffix so it's obvious they belong
# to the same run: dvs_repo_cli_AbC / dvs_storage_cli_AbC / dvs_repo_rpkg_AbC /
# dvs_storage_rpkg_AbC.

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
DVS_REPO_RPKG="$SCRIPT_DIR/dvs_repo_rpkg_$RUN_SUFFIX"
DVS_STORAGE_RPKG="$SCRIPT_DIR/dvs_storage_rpkg_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI" "$DVS_REPO_RPKG" "$DVS_STORAGE_RPKG"

# ── Init ──

cd "$DVS_REPO_CLI"
dvs init "$DVS_STORAGE_CLI"

cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_RPKG")
EOF

# ── Create nested file tree in CLI repo ──
# Layout:
#   top_a.bin                  top-level, will be tracked
#   top_b.bin                  top-level, intentionally NEVER tracked
#   data/shallow_1.bin         direct children of data/
#   data/shallow_2.bin
#   data/raw/deep_1.bin        one level deeper
#   data/raw/deep_2.bin
#   data/derived/deep_3.bin    one level deeper, sibling subdir
#   models/v1/model_1.bin      second top-level tree
#   models/v1/model_2.bin

cd "$DVS_REPO_CLI"

mkrandfile top_a.bin
mkrandfile top_b.bin
mkrandfile data/shallow_1.bin
mkrandfile data/shallow_2.bin
mkrandfile data/raw/deep_1.bin
mkrandfile data/raw/deep_2.bin
mkrandfile data/derived/deep_3.bin
mkrandfile models/v1/model_1.bin
mkrandfile models/v1/model_2.bin

say "--- tree (CLI repo fixture: 9 files across data/{,raw,derived} + models/v1 + top-level) ---"
tree --noreport

# ── Create identical nested file tree in R repo ──

cd "$DVS_REPO_RPKG"

mkrandfile top_a.bin
mkrandfile top_b.bin
mkrandfile data/shallow_1.bin
mkrandfile data/shallow_2.bin
mkrandfile data/raw/deep_1.bin
mkrandfile data/raw/deep_2.bin
mkrandfile data/derived/deep_3.bin
mkrandfile models/v1/model_1.bin
mkrandfile models/v1/model_2.bin

say "--- tree (R repo fixture: identical layout) ---"
tree --noreport

# ══════════════════════════════════════════════════════════════════════════════
# ── 1. ADD ──
# dvs add has no --recursive flag. Recursion is encoded in the glob.
# GlobBuilder::literal_separator(true) means *.bin matches only one level;
# **/*.bin recurses into subdirectories.
# ══════════════════════════════════════════════════════════════════════════════

# ── 1.A  dvs add data/ — bare directory, no glob → no-op ──

say
say "=== CLI 1.A: dvs add data/ (bare directory — no glob match → error, nothing tracked) ==="
cd "$DVS_REPO_CLI"
dvs add data/ || echo "(expected: no files matched — bare directory without glob)"

say
say "=== CLI 1.A: dvs status after bare-dir add (should show zero tracked files) ==="
dvs status

# ── 1.A.R  R mirror ──

say
say "=== R 1.A.R: dvs_add(paths = 'data/') (bare directory — expected no-op/error) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
tryCatch(
  dvs_add(paths = "data/"),
  error = function(e) message("(expected: no files matched — ", conditionMessage(e), ")")
)
EOF

say
say "=== R 1.A.R: dvs_status() after bare-dir add (should show zero tracked files) ==="
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 1.B  dvs add data/ --recursive — DEMO: the flag belongs to get/status, not add ──
# `--recursive` is on `dvs get` and `dvs status` only. The clap rejection that
# follows is the demonstration, not a script bug.

say
say "=== CLI 1.B: \`--recursive\` is valid on get/status; \`dvs add\` rejects it. ==="
say "    The clap error on the next line is the demo (the script absorbs the failure)."
cd "$DVS_REPO_CLI"
dvs add data/ --recursive || echo "(confirmed: --recursive is not a flag on \`dvs add\`)"

# ── 1.B.R  R mirror: dvs_add() has no `recursive` parameter — unused-argument error ──

say
say "=== R 1.B.R: dvs_add() has no \`recursive\` parameter; passing one is an unused-argument error. ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
tryCatch(
  dvs_add(paths = "data/", recursive = TRUE),
  error = function(e) message("(confirmed: ", conditionMessage(e), ")")
)
EOF

# ── 1.C  dvs add --glob 'data/*.bin' — literal separator: shallow only ──

say
say "=== CLI 1.C: dvs add --glob 'data/*.bin' (literal-separator: shallow_*.bin only) ==="
cd "$DVS_REPO_CLI"
dvs add --glob 'data/*.bin'

say
say "=== CLI 1.C: dvs status (should show 2 files: shallow_1.bin + shallow_2.bin) ==="
dvs status

# ── 1.C.R ──

say
say "=== R 1.C.R: dvs_add(glob = 'data/*.bin') (shallow only) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_add(glob = "data/*.bin")
EOF

say
say "=== R 1.C.R: dvs_status() (should show 2 files) ==="
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 1.D  dvs add --glob 'data/**/*.bin' — recursive: deep files too ──

say
say "=== CLI 1.D: dvs add --glob 'data/**/*.bin' (recursive glob — adds raw/* and derived/*) ==="
cd "$DVS_REPO_CLI"
dvs add --glob 'data/**/*.bin'

say
say "=== CLI 1.D: dvs status (should show 5 files under data/) ==="
dvs status

# ── 1.D.R ──

say
say "=== R 1.D.R: dvs_add(glob = 'data/**/*.bin') (recursive glob) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_add(glob = "data/**/*.bin")
EOF

say
say "=== R 1.D.R: dvs_status() (should show 5 files under data/) ==="
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 1.E  dvs add --glob 'models/**/*.bin' — second tree ──

say
say "=== CLI 1.E: dvs add --glob 'models/**/*.bin' (second tree) ==="
cd "$DVS_REPO_CLI"
dvs add --glob 'models/**/*.bin'

say
say "=== CLI 1.E: dvs status (should show 7 files: 5 data/ + 2 models/) ==="
dvs status

# ── 1.E.R ──

say
say "=== R 1.E.R: dvs_add(glob = 'models/**/*.bin') (second tree) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_add(glob = "models/**/*.bin")
EOF

say
say "=== R 1.E.R: dvs_status() (should show 7 files) ==="
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 1.F  dvs add top_a.bin — explicit file; top_b.bin left untracked ──

say
say "=== CLI 1.F: dvs add top_a.bin (explicit; top_b.bin intentionally omitted) ==="
cd "$DVS_REPO_CLI"
dvs add top_a.bin

# ── 1.F.R ──

say
say "=== R 1.F.R: dvs_add(paths = 'top_a.bin') ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_add(paths = "top_a.bin")
EOF

# ── 1.G  Final status: all 8 tracked files ──

say
say "=== CLI 1.G: dvs status (final — 8 tracked: top_a + 2 shallow + 2 raw + 1 derived + 2 models) ==="
cd "$DVS_REPO_CLI"
dvs status

say
say "=== R 1.G.R: dvs_status() (final — 8 tracked) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ══════════════════════════════════════════════════════════════════════════════
# ── 2. GET ──
# Both repos are now fully tracked. Delete data/* so files show absent,
# then demonstrate non-recursive vs recursive get.
# ══════════════════════════════════════════════════════════════════════════════

# ── 2.A  dvs get data/ — non-recursive: shallow_*.bin only ──

say
say "=== CLI 2.A: deleting data/ files, then dvs get data/ (non-recursive) ==="
cd "$DVS_REPO_CLI"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin

dvs get data/

say "--- tree data (expect shallow_*.bin only; raw/ + derived/ empty) ---"
tree --noreport data

# ── 2.A.R ──

say
say "=== R 2.A.R: deleting data/ files, then dvs_get(paths = 'data/') (non-recursive) ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get(paths = "data/")
EOF

say "--- R 2.A.R tree data (expect shallow_*.bin only; raw/ + derived/ empty) ---"
tree --noreport data

# ── 2.A dry-run pair (R only) — makes the difference visible without disk churn ──

say
say "=== R 2.A.DR: dry_run comparison: non-recursive vs recursive ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin

print_eval_rscript <<EOF
library(dvs)
cat("-- dry_run non-recursive --\n")
dvs_get(paths = "data/", dry_run = TRUE)
cat("-- dry_run recursive --\n")
dvs_get(paths = "data/", recursive = TRUE, dry_run = TRUE)
EOF

# ── 2.B  dvs get --recursive data/ — all five data/ files restored ──

say
say "=== CLI 2.B: wipe shallow_*.bin, then dvs get data/ --recursive ==="
cd "$DVS_REPO_CLI"
rm -f data/shallow_1.bin data/shallow_2.bin

dvs get data/ --recursive

say "--- tree data (expect shallow_*.bin + raw/deep_*.bin + derived/deep_3.bin) ---"
tree --noreport data

# ── 2.B.R ──

say
say "=== R 2.B.R: wipe shallow_*.bin, then dvs_get(paths = 'data/', recursive = TRUE) ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get(paths = "data/", recursive = TRUE)
EOF

say "--- R 2.B.R tree data (expect shallow_*.bin + raw/deep_*.bin + derived/deep_3.bin) ---"
tree --noreport data

# ── 2.C  dvs get (no path) — restores everything regardless of --recursive ──

say
say "=== CLI 2.C: wipe all tracked files, then dvs get --glob '**/*.bin' (everything, glob) ==="
cd "$DVS_REPO_CLI"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

dvs get --glob '**/*.bin'

say "--- tree (expect every tracked file restored at every depth) ---"
tree --noreport

# ── 2.C.R ──

say
say "=== R 2.C.R: wipe all tracked files, then dvs_get(glob = '**/*.bin') (everything, glob) ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get(glob = "**/*.bin")
EOF

say "--- R 2.C.R tree (expect every tracked file restored at every depth) ---"
tree --noreport

# ── 2.D  No paths AND no glob — CLI suggests, R restores everything ──
# Asymmetric on purpose: `dvs get` (CLI) refuses with a suggestion to use
# `--glob '**/*'`, since restoring everything by accident is a footgun on
# the command line. `dvs_get()` (R) is for interactive use — defaults to
# the everything case, deferring to `resolve_paths_for_get`'s empty-paths
# semantics (no filter, every tracked metadata entry passes, all depths).

say
say "=== CLI 2.D: dvs get (no args — should refuse with a suggestion) ==="
cd "$DVS_REPO_CLI"
dvs get || echo "(exit nonzero as expected)"

say
say "=== R 2.D.R: dvs_get() (no args — restore everything, including 2-level-deep files) ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get()
EOF

say "--- R 2.D.R tree (expect every tracked file restored at every depth) ---"
tree --noreport

# ── 2.F  dvs get --glob '**/*' — extension-agnostic "get everything" ──
# Same spirit as 2.C, but without the `.bin` extension restriction. With
# `**/*` the glob matches any tracked path at any depth, and since only
# tracked files are candidates anyway, this is the cleanest spelling for
# "restore the whole repo".

say
say "=== CLI 2.F: wipe all tracked files, then dvs get --glob '**/*' (extension-agnostic) ==="
cd "$DVS_REPO_CLI"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

dvs get --glob '**/*'

say "--- tree (expect every tracked file restored at every depth) ---"
tree --noreport

say
say "=== R 2.F.R: dvs_get(glob = '**/*') (extension-agnostic) ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get(glob = "**/*")
EOF

say "--- R 2.F.R tree (expect every tracked file restored at every depth) ---"
tree --noreport

# ── 2.E  dvs get . --recursive — path-based "get everything" ──
# `.` is normalized via `normalize_path` (stripping the CurDir component) to
# an empty PathBuf, which `Path::starts_with` treats as a prefix of every
# tracked path. So `dvs get . --recursive` and `dvs_get(paths = ".",
# recursive = TRUE)` restore every tracked file at every depth, the same
# set as 2.C / 2.F.

say
say "=== CLI 2.E: wipe all tracked files, then dvs get . --recursive (path-based 'get everything') ==="
cd "$DVS_REPO_CLI"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

dvs get . --recursive

say "--- tree (expect every tracked file restored at every depth) ---"
tree --noreport

say
say "=== R 2.E.R: dvs_get(paths = '.', recursive = TRUE) (path-based 'get everything') ==="
cd "$DVS_REPO_RPKG"
rm -f data/shallow_1.bin data/shallow_2.bin data/raw/deep_1.bin data/raw/deep_2.bin data/derived/deep_3.bin top_a.bin models/v1/model_1.bin models/v1/model_2.bin

print_eval_rscript <<EOF
library(dvs)
dvs_get(paths = ".", recursive = TRUE)
EOF

say "--- R 2.E.R tree (expect every tracked file restored at every depth) ---"
tree --noreport

# ══════════════════════════════════════════════════════════════════════════════
# ── 3. STATUS ──
# All files current on disk. Demonstrate shallow vs recursive vs no-filter.
# ══════════════════════════════════════════════════════════════════════════════

# ── 3.A  dvs status data/ — shallow only (2 files) ──

say
say "=== CLI 3.A: dvs status data/ (non-recursive — shallow_*.bin only, expect 2 rows) ==="
cd "$DVS_REPO_CLI"
dvs status data/

say
say "=== R 3.A.R: dvs_status(paths = 'data/') (non-recursive, expect 2 rows) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(paths = "data/")
EOF

# ── 3.B  dvs status --recursive data/ — all 5 under data/ ──

say
say "=== CLI 3.B: dvs status data/ --recursive (recursive — all under data/, expect 5 rows) ==="
cd "$DVS_REPO_CLI"
dvs status data/ --recursive

say
say "=== R 3.B.R: dvs_status(paths = 'data/', recursive = TRUE) (expect 5 rows) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(paths = "data/", recursive = TRUE)
EOF

# ── 3.C  dvs status (no filter) — all 8 tracked files at every depth ──
# `--recursive` is NOT needed here. With no explicit path, the filter is
# disabled entirely and every tracked file is returned, including the
# `data/raw/deep_*.bin`, `data/derived/deep_3.bin`, and `models/v1/model_*.bin`
# files that live two levels under cwd. The flag only constrains descendants
# of paths the user passes explicitly (3.A vs 3.B).

say
say "=== CLI 3.C: dvs status (no filter — all 8 tracked, INCLUDING deep_*.bin and models/v1/model_*.bin two levels down) ==="
cd "$DVS_REPO_CLI"
dvs status

say
say "=== R 3.C.R: dvs_status() (no filter — all 8 tracked, including 2-level-deep files) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status()
EOF

# ── 3.D  Mid-tree directory (data/raw/) — non-recursive: 2 rows ──

say
say "=== CLI 3.D: dvs status data/raw/ (mid-tree dir, non-recursive — expect 2 rows: deep_1, deep_2) ==="
cd "$DVS_REPO_CLI"
dvs status data/raw/

say
say "=== R 3.D.R: dvs_status(paths = 'data/raw/') (expect 2 rows) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(paths = "data/raw/")
EOF

# ── 3.E  Multiple explicit paths — non-recursive each ──

say
say "=== CLI 3.E: dvs status data/raw/ data/derived/ (multi-path, non-recursive — expect 3 rows) ==="
cd "$DVS_REPO_CLI"
dvs status data/raw/ data/derived/

say
say "=== R 3.E.R: dvs_status(paths = c('data/raw/', 'data/derived/')) (expect 3 rows) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(paths = c("data/raw/", "data/derived/"))
EOF

# ── 3.F  --recursive without a path — flag is irrelevant when paths is empty ──
# Same 8 rows as 3.C: the `recursive` flag only scopes descendants of explicit
# paths. With no path, the filter is None either way and every tracked file
# (including deep-nested ones) is returned.

say
say "=== CLI 3.F: dvs status --recursive (no path — flag is a no-op, expect 8 rows, identical to 3.C) ==="
cd "$DVS_REPO_CLI"
dvs status --recursive  # no positional, so order is moot

say
say "=== R 3.F.R: dvs_status(recursive = TRUE) (no path — flag is a no-op, expect 8 rows) ==="
cd "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_status(recursive = TRUE)
EOF

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
