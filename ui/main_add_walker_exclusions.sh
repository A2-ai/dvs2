#!/usr/bin/env bash
#
# Showcase: glob-walk exclusions in add (.git, dvs.toml, .gitignore, broken symlinks).
#
# Every case here was verified broken on pre-fix main (2026-07-16):
#
#   A. a broken symlink inside a walked directory aborted the WHOLE add with
#      "Error: No such file or directory (os error 2)". The valid files next
#      to it were not added.
#   B. `dvs add . --glob '**/*'` tracked the repository's internals: 19 .dvs
#      sidecars under .dvs/.git (config, HEAD, hooks, info/exclude) plus
#      dvs.toml itself and any .gitignore files.
#
# With the fix, the walker skips .git entirely, skips dvs.toml and .gitignore
# by name, and skips broken symlinks with a warning instead of failing.
# Explicitly named paths are NOT filtered. Only glob walks are. That mirrors
# the explicit-strict versus glob-lenient policy adopted for symlinks in #274.
#
# The R binding routes through the same resolver, so a parity section asserts
# the same outcome via dvs_add(glob = ...).

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-all\` should have been run first so the dvs CLI and the installed dvs R package both reflect this branch."

# region: SETUP

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_walker_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_walker_$RUN_SUFFIX"
DVS_REPO_RPKG="$SCRIPT_DIR/dvs_repo_rpkg_walker_$RUN_SUFFIX"
DVS_STORAGE_RPKG="$SCRIPT_DIR/dvs_storage_rpkg_walker_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI" "$DVS_REPO_RPKG" "$DVS_STORAGE_RPKG"

seed_repo() { # $1 = repo dir
  mkdir -p "$1/data/subdir"
  echo "a" > "$1/data/a.csv"
  echo "b" > "$1/data/b.csv"
  echo "c" > "$1/data/subdir/c.csv"
  echo "*.log" > "$1/.gitignore"
  echo "*.tmp" > "$1/data/.gitignore"
  ln -s /nonexistent-target "$1/data/broken.csv"
}

cd "$DVS_REPO_CLI"
git init -q .
dvs init "$DVS_STORAGE_CLI"
seed_repo "$DVS_REPO_CLI"
say "--- tree (data files, .gitignore files, one broken symlink) ---"
tree -a -I .git --noreport

# region: A — broken symlink no longer aborts the walk

say
say "============================================================"
say "= A: add data --glob '*.csv' with a broken symlink present ="
say "============================================================"
# Pre-fix: exit 1, 'No such file or directory', nothing added.
dvs add data --glob '*.csv'
test -f .dvs/data/a.csv.dvs
test -f .dvs/data/b.csv.dvs
test ! -e .dvs/data/broken.csv.dvs
say "OK: both real files added, broken symlink skipped, batch survived"

# region: B — glob-everything does not sweep repo internals

say
say "============================================================"
say "= B: add . --glob '**/*' skips .git, dvs.toml, .gitignore  ="
say "============================================================"
# Pre-fix: this tracked .dvs/.git/config.dvs, hooks, dvs.toml.dvs, ...
dvs add . --glob '**/*'
test -f .dvs/data/subdir/c.csv.dvs
test ! -e .dvs/.git
test ! -e .dvs/dvs.toml.dvs
test ! -e .dvs/.gitignore.dvs
test ! -e .dvs/data/.gitignore.dvs
say "OK: real files tracked, no .git/dvs.toml/.gitignore sidecars"

# region: C — explicit paths are intentionally NOT filtered

say
say "============================================================"
say "= C: explicitly named .gitignore is still addable          ="
say "============================================================"
# The exclusions apply to glob WALKS only. Naming a path explicitly is user
# intent and stays strict, same as the #274 symlink policy.
dvs add .gitignore
test -f .dvs/.gitignore.dvs
say "OK: explicit add of .gitignore still works"

# region: D — R binding parity

say
say "============================================================"
say "= D: dvs_add(glob) parity through the R binding            ="
say "============================================================"
cd "$DVS_REPO_RPKG"
git init -q .
seed_repo "$DVS_REPO_RPKG"
print_eval_rscript <<EOF
library(dvs)
dvs_init("$DVS_STORAGE_RPKG")
result <- dvs_add(glob = "**/*")
print(result, width = Inf)
paths <- as.character(result\$path)
stopifnot(
  "data/a.csv" %in% paths,
  "data/subdir/c.csv" %in% paths,
  !any(grepl("^\\\\.git/", paths)),
  !any(grepl("\\\\.gitignore$", paths)),
  !"dvs.toml" %in% paths,
  !any(grepl("broken", paths))
)
cat("OK: R binding resolves the same file set\n")
EOF

say
say "All walker-exclusion cases passed."
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
