#!/usr/bin/env bash

# Script 7 — validate_global.sh
#
# Validates cross-cutting CLI surface claims from specs.md (CLI only):
#   - `dvs --help` subcommand order: init, add, status, get, help (L27)
#   - `--json` accepted by all four subcommands init/add/status/get (L13)
#   - project root discovery: walk UP from a nested subdir to dvs.toml (L19-20)
#   - multiple projects in one git repo: nearest dvs.toml wins (L20, L71)
#   - gitignore entry format `/<filename>`, no duplicate on re-add (L425-426)
#   - gitignore skipped when no `.git` folder, add still succeeds (L426-427)
#   - parallelism: DVS_NUM_THREADS honored (L412) [count untestable via CLI]
#
# Each claim prints expected vs actual and a PASS/FAIL/UNTESTABLE verdict.
# Ends with a `=== SUMMARY: N pass, M fail ===` line.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# ── verdict helpers ──────────────────────────────────────────────────────────
PASS_N=0
FAIL_N=0
UNTESTABLE_N=0

check() {
  { set +x; } 2>/dev/null
  # check WANT GOT LABEL
  if [ "$1" = "$2" ]; then
    echo "PASS: $3 (want=$1 got=$2)"
    PASS_N=$((PASS_N + 1))
  else
    echo "FAIL: $3 (want=$1 got=$2)"
    FAIL_N=$((FAIL_N + 1))
  fi
  { set -x; } 2>/dev/null
}

untestable() {
  { set +x; } 2>/dev/null
  echo "UNTESTABLE: $1"
  UNTESTABLE_N=$((UNTESTABLE_N + 1))
  { set -x; } 2>/dev/null
}

# ── setup: temp repo + sibling storage ───────────────────────────────────────
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 1: dvs --help subcommand order init, add, status, get, help (L27) ==="
# Extract the subcommand names from the Commands: section, in order.
# clap lists subcommands one per line, two-space-indented, in the Commands block.
HELP_OUT="$(dvs --help)"
ORDER="$(printf '%s\n' "$HELP_OUT" \
  | awk '/^Commands:/{f=1;next} f && /^[[:space:]]+[a-z]/{print $1} f && /^$/{exit}' \
  | paste -sd, -)"
check "init,add,status,get,help" "$ORDER" "dvs --help subcommand order"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 2: --json accepted by all four subcommands init/add/status/get (L13) ==="
# Each must emit JSON (starts with [ or {), not a clap/usage error.
# init --json: use a throwaway nested project so we do not clobber the main repo.
JSON_DIR="$DVS_REPO/jsonprobe"
JSON_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_json"
mkdir -p "$JSON_DIR" "$JSON_STORE"
INIT_JSON_RC=0
INIT_JSON="$(cd "$JSON_DIR" && dvs init "$JSON_STORE" --json --root-dir "$(pwd)" 2>&1)" || INIT_JSON_RC=$?
case "$INIT_JSON" in
  \[*|\{*) INIT_JSON_OK=yes ;;
  *)       INIT_JSON_OK=no  ;;
esac
check "yes" "$INIT_JSON_OK" "init --json emits JSON (rc=$INIT_JSON_RC)"

# add --json
head -c 1024 /dev/urandom > "$DVS_REPO/g.bin"
ADD_JSON="$(cd "$DVS_REPO" && dvs add g.bin --json 2>&1)"
case "$ADD_JSON" in
  \[*|\{*) ADD_JSON_OK=yes ;;
  *)       ADD_JSON_OK=no  ;;
esac
check "yes" "$ADD_JSON_OK" "add --json emits JSON"

# status --json
STATUS_JSON="$(cd "$DVS_REPO" && dvs status --json 2>&1)"
case "$STATUS_JSON" in
  \[*|\{*) STATUS_JSON_OK=yes ;;
  *)       STATUS_JSON_OK=no  ;;
esac
check "yes" "$STATUS_JSON_OK" "status --json emits JSON"

# get --json (delete local copy first so get does real work)
rm -f "$DVS_REPO/g.bin"
GET_JSON="$(cd "$DVS_REPO" && dvs get g.bin --json 2>&1)"
case "$GET_JSON" in
  \[*|\{*) GET_JSON_OK=yes ;;
  *)       GET_JSON_OK=no  ;;
esac
check "yes" "$GET_JSON_OK" "get --json emits JSON"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 3: project root discovery walks UP from nested subdir (L19-20) ==="
mkdir -p "$DVS_REPO/data/raw"
head -c 1024 /dev/urandom > "$DVS_REPO/data/raw/nested.bin"
(cd "$DVS_REPO" && dvs add data/raw/nested.bin >/dev/null)
# From data/raw (depth 2) run status; must find project root and list the file.
NESTED_RC=0
NESTED_OUT="$(cd "$DVS_REPO/data/raw" && dvs status --json 2>&1)" || NESTED_RC=$?
# Paths are reported relative to project root, so expect data/raw/nested.bin.
case "$NESTED_OUT" in
  *'"path":"data/raw/nested.bin"'*) NESTED_OK=yes ;;
  *)                                NESTED_OK=no  ;;
esac
check "yes" "$NESTED_OK" "status from data/raw/ resolves project root (rc=$NESTED_RC)"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 4: nested dvs.toml resolves to the NEAREST project (L20, L71) ==="
# Inside the existing repo create a nested project with its OWN dvs.toml + storage.
NESTED_PROJ="$DVS_REPO/inner"
NESTED_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_inner"
mkdir -p "$NESTED_PROJ" "$NESTED_STORE"
(cd "$NESTED_PROJ" && dvs init "$NESTED_STORE" --root-dir "$(pwd)" >/dev/null)
# init nested below parent dvs.toml must succeed (local check, L71).
check "1" "$([ -f "$NESTED_PROJ/dvs.toml" ] && echo 1 || echo 0)" "nested init succeeds despite parent dvs.toml"
# Add a file from inside the nested project; its blob must land in NESTED_STORE,
# proving dvs resolved the nearest (inner) dvs.toml, not the outer one.
head -c 777 /dev/urandom > "$NESTED_PROJ/inner.bin"
(cd "$NESTED_PROJ" && dvs add inner.bin --json >/dev/null)
INNER_HASH="$(cd "$NESTED_PROJ" && dvs status --json | sed -n 's/.*"blake3":"\([0-9a-f]\{64\}\)".*/\1/p' | head -1)"
INNER_BLOB="$NESTED_STORE/${INNER_HASH:0:2}/${INNER_HASH:2}"
OUTER_BLOB="$DVS_STORAGE/${INNER_HASH:0:2}/${INNER_HASH:2}"
if [ -f "$INNER_BLOB" ] && [ ! -f "$OUTER_BLOB" ]; then
  NEAREST=nearest
elif [ -f "$OUTER_BLOB" ]; then
  NEAREST=outer
else
  NEAREST=neither
fi
check "nearest" "$NEAREST" "add from inner/ stores in NEAREST storage (inner), not outer"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 5: gitignore entry format /<filename>, no duplicate on re-add (L425-426) ==="
GI="$DVS_REPO/data/raw/.gitignore"
GI_CONTENT="$(cat "$GI" 2>/dev/null || true)"
# Expect a line exactly '/nested.bin' in the file's OWN directory.
if printf '%s\n' "$GI_CONTENT" | grep -qx '/nested.bin'; then GI_FMT=yes; else GI_FMT=no; fi
check "yes" "$GI_FMT" "data/raw/.gitignore contains '/nested.bin'"
# Re-add and confirm no duplicate entry.
(cd "$DVS_REPO" && dvs add data/raw/nested.bin >/dev/null 2>&1)
GI_COUNT="$(grep -cx '/nested.bin' "$GI" 2>/dev/null || echo 0)"
check "1" "$GI_COUNT" "no duplicate /nested.bin entry after re-add"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 6: no .git -> gitignore skipped, add still succeeds (L426-427) ==="
NOGIT_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_nogit_XXX)"
NOGIT_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_nogit"
mkdir "$NOGIT_STORE"
# Deliberately NO `git init` here.
(cd "$NOGIT_REPO" && dvs init "$NOGIT_STORE" >/dev/null)
head -c 1024 /dev/urandom > "$NOGIT_REPO/b.bin"
NOGIT_RC=0
(cd "$NOGIT_REPO" && dvs add b.bin >/dev/null 2>&1) || NOGIT_RC=$?
check "0" "$NOGIT_RC" "add succeeds in repo with no .git"
# No .gitignore should have been written anywhere in the repo.
NOGIT_GI_COUNT="$(find "$NOGIT_REPO" -name .gitignore | wc -l | tr -d ' ')"
check "0" "$NOGIT_GI_COUNT" "no .gitignore written when no .git folder"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 7: parallelism DVS_NUM_THREADS honored (L412) ==="
# The actual thread count is not surfaced in plain CLI output, so the COUNT
# itself is UNTESTABLE via the CLI. What we CAN verify: setting the variable
# does not error and still produces correct results.
mkdir -p "$DVS_REPO/par"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p1.bin"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p2.bin"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p3.bin"
PAR_RC=0
PAR_OUT="$(cd "$DVS_REPO" && DVS_NUM_THREADS=2 dvs add par/p1.bin par/p2.bin par/p3.bin --json 2>&1)" || PAR_RC=$?
# Correct result: 3 copied outcomes.
PAR_COPIED="$(printf '%s' "$PAR_OUT" | grep -o '"outcome":"copied"' | wc -l | tr -d ' ')"
check "0" "$PAR_RC" "DVS_NUM_THREADS=2 dvs add does not error"
check "3" "$PAR_COPIED" "DVS_NUM_THREADS=2 add produces correct results (3 copied)"
untestable "DVS_NUM_THREADS actual thread COUNT (L412) not observable via plain CLI output"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS_N pass, $FAIL_N fail ==="
say "(also $UNTESTABLE_N untestable)"

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
