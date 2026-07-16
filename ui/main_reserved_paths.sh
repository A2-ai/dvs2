#!/usr/bin/env bash
#
# Showcase: reserved paths (.git, the metadata folder) in the tracked set and
# in explicit adds.
#
# Every case here was verified broken on pre-fix main (2026-07-17):
#
#   A. Sidecars are committed to git, so a repo that ever ran a pre-exclusion
#      `dvs add . --glob '**/*'` carries sidecars for .git internals in every
#      clone. `dvs status` listed those paths as tracked and `dvs get . -r`
#      RESTORED them: overwriting .git/refs/heads/main silently rolled the
#      branch back and orphaned the newest commit, with a clean `git status`.
#   B. `dvs add .git/config` was accepted and tracked a git internal.
#   C. `dvs add .dvs/<file>.dvs` was accepted: it created a sidecar of a
#      sidecar under .dvs/.dvs/ and gitignored the original sidecar, the one
#      file that MUST stay committed.
#
# With the fix, tracked-set derivation skips reserved sidecars with a warning
# (get and status share that choke point, so even poisoned repos and their
# clones are safe), and explicit adds of reserved paths are validator-level
# rejections that refuse the batch. Explicitly adding .gitignore or dvs.toml
# stays allowed, that boundary is deliberate.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-cli\` should have been run first so the dvs CLI reflects this branch."

# region: SETUP

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_reserved_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_reserved_$RUN_SUFFIX"
DVS_REPO_EXPLICIT="$SCRIPT_DIR/dvs_repo_cli_reserved_explicit_$RUN_SUFFIX"
DVS_STORAGE_EXPLICIT="$SCRIPT_DIR/dvs_storage_cli_reserved_explicit_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI" "$DVS_REPO_EXPLICIT" "$DVS_STORAGE_EXPLICIT"

git_commit() { # $1 = repo dir, $2 = message
  git -C "$1" -c user.name=ui -c user.email=ui@test -c commit.gpgsign=false commit -qm "$2"
}

# region: A — poisoned-repo rescue: get/status ignore reserved sidecars

say
say "============================================================"
say "= A: poisoned repo, get . -r must not roll the branch back ="
say "============================================================"
cd "$DVS_REPO_CLI"
git init -q -b main .
dvs init "$DVS_STORAGE_CLI"
mkdir -p data
printf 'sepal,petal\n5.1,1.4\n' > data/iris.csv
dvs add data/iris.csv
git add .
git_commit "$DVS_REPO_CLI" "add data"

# Simulate the poison a pre-exclusion glob add left behind, without depending
# on a pre-fix binary: track a copy of the current branch ref, then reuse its
# sidecar (valid metadata, blob in storage) as .dvs/.git/refs/heads/main.dvs.
# A get that acts on it restores the OLD ref over the branch. Also plant a
# sidecar of a sidecar, the .dvs/.dvs shape a swept metadata folder produced.
cp .git/refs/heads/main refcapture.bin
dvs add refcapture.bin
mkdir -p .dvs/.git/refs/heads
cp .dvs/refcapture.bin.dvs .dvs/.git/refs/heads/main.dvs
mkdir -p .dvs/.dvs/data
cp .dvs/data/iris.csv.dvs .dvs/.dvs/data/iris.csv.dvs.dvs
git add .
git_commit "$DVS_REPO_CLI" "poison: sidecars for reserved paths"

echo "second subject" > README.md
git add README.md
git_commit "$DVS_REPO_CLI" "second subject"
git log --oneline
HEAD_BEFORE="$(git rev-parse HEAD)"
test "$(git rev-list --count HEAD)" -eq 3

# The innocent restore: a collaborator pulls the poisoned repo and asks dvs
# for their data back. Pre-fix this overwrote .git/refs/heads/main with the
# older ref and the newest commit vanished from git log.
rm data/iris.csv
RUST_LOG=warn dvs get . -r

git log --oneline
test "$(git rev-list --count HEAD)" -eq 3
test "$(git rev-parse HEAD)" = "$HEAD_BEFORE"
git log --oneline | grep "second subject"
test "$(cat .git/refs/heads/main)" = "$HEAD_BEFORE"
test -f data/iris.csv
say "OK: data restored, branch ref untouched, newest commit intact"

rc=0
dvs status | tee /dev/stderr | grep -E '\.git/|\.dvs/' || rc=$?
test "$rc" -ne 0
say "OK: status lists no .git or metadata-folder paths despite the stale sidecars"

# region: B — explicit add of a .git internal is rejected

say
say "============================================================"
say "= B: dvs add .git/config is refused                        ="
say "============================================================"
cd "$DVS_REPO_EXPLICIT"
git init -q -b main .
dvs init "$DVS_STORAGE_EXPLICIT"
mkdir -p data
printf 'sepal,petal\n5.1,1.4\n' > data/iris.csv

rc=0
dvs add .git/config || rc=$?
test "$rc" -ne 0
test ! -e .dvs/.git
say "OK: explicit .git path rejected, exit $rc, no sidecar written"

# region: C — explicit add under the metadata folder is rejected

say
say "============================================================"
say "= C: dvs add .dvs/<sidecar>.dvs is refused                 ="
say "============================================================"
dvs add data/iris.csv
test -f .dvs/data/iris.csv.dvs

rc=0
dvs add .dvs/data/iris.csv.dvs || rc=$?
test "$rc" -ne 0
test ! -e .dvs/.dvs
# The real sidecar must not have been gitignored by the refused add.
rc=0
grep -r 'iris.csv.dvs' .dvs/data/.gitignore .gitignore 2>/dev/null || rc=$?
test "$rc" -ne 0
say "OK: metadata-folder path rejected, no sidecar of a sidecar, sidecar not gitignored"

# region: D — .gitignore and dvs.toml stay explicitly addable

say
say "============================================================"
say "= D: explicit .gitignore and dvs.toml are still addable    ="
say "============================================================"
# The reserved set is .git and the metadata folder only. Naming .gitignore or
# dvs.toml explicitly is user intent and keeps working, same boundary as the
# glob-walk exclusions.
echo "*.log" > .gitignore
dvs add .gitignore
test -f .dvs/.gitignore.dvs
dvs add dvs.toml
test -f .dvs/dvs.toml.dvs
say "OK: explicit adds of .gitignore and dvs.toml still work"

say
say "All reserved-path cases passed."
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
