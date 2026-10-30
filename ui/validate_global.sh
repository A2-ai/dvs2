#!/usr/bin/env bash

# Script 7 — validate_global.sh
#
# Validates cross-cutting CLI surface claims from specs.md (CLI only).
# Line refs (Lnnn) are to specs.md on main at 370f448.
#   - `dvs --help` lists every subcommand the spec documents (L31, L131-229, L270-666)
#   - `--json` accepted by every subcommand that runs locally, before or after it (L13, L82)
#   - a --json argument error is an envelope with command `cli`, exit 2 (L89-90, L216)
#   - project root discovery: walk UP from a nested subdir to dvs.toml (L23-24)
#   - multiple projects in one git repo: nearest dvs.toml wins (L24, L273)
#   - gitignore entry format `/<filename>`, no duplicate on re-add (L791-792)
#   - gitignore skipped when no `.git` folder, add still succeeds (L792-793)
#   - parallelism: DVS_NUM_THREADS honored (L778-782) [count untestable via CLI]
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

# Run a python expression over the --json envelope on stdin. Prints
# <parse-error> instead of aborting, so the script always reaches SUMMARY.
jq_py() { python3 -c "import sys,json;d=json.load(sys.stdin);$1" 2>/dev/null || echo "<parse-error>"; }

# "v1 <command>" when stdin is one v1 envelope (L89-90), else what it is.
envelope_of() { jq_py 'print("v%s %s" % (d.get("v"), d.get("command")))'; }

# ── setup: temp repo + sibling storage ───────────────────────────────────────
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init local "$DVS_STORAGE"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 1: dvs --help lists every subcommand the spec documents ==="
# Main's specs.md has no `dvs --help` block, so it fixes no order. It documents
# init (L270), attach (L184), add (L379), status (L601), get (L451), info
# (L189), availability (L131), history (L133), login and logout (L221-229).
# clap appends help. Compare the sets.
HELP_OUT="$(dvs --help)"
COMMANDS="$(printf '%s\n' "$HELP_OUT" \
  | awk '/^Commands:/{f=1;next} f && /^[[:space:]]+[a-z]/{print $1} f && /^$/{exit}' \
  | sort | paste -sd, -)"
check "add,attach,availability,get,help,history,info,init,login,logout,status" "$COMMANDS" "dvs --help subcommand set"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 2: --json accepted by every subcommand that runs locally (L13, L82, L89-90) ==="
# Each must print one v1 envelope naming its command on stdout (stderr is for
# diagnostics, L84), not a clap/usage error.
# init --json: use a throwaway nested project so we do not clobber the main repo.
JSON_DIR="$DVS_REPO/jsonprobe"
JSON_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_json"
mkdir -p "$JSON_DIR" "$JSON_STORE"
INIT_JSON_RC=0
INIT_JSON="$(cd "$JSON_DIR" && dvs init local "$JSON_STORE" --json --root-dir "$(pwd)")" || INIT_JSON_RC=$?
check "v1 init" "$(printf '%s' "$INIT_JSON" | envelope_of)" "init --json emits a v1 envelope (rc=$INIT_JSON_RC)"

# add --json
head -c 1024 /dev/urandom > "$DVS_REPO/g.bin"
ADD_JSON="$(cd "$DVS_REPO" && dvs add g.bin --json)"
check "v1 add" "$(printf '%s' "$ADD_JSON" | envelope_of)" "add --json emits a v1 envelope"

# status --json, and --json before the subcommand (global, L82)
STATUS_JSON="$(cd "$DVS_REPO" && dvs status --json)"
check "v1 status" "$(printf '%s' "$STATUS_JSON" | envelope_of)" "status --json emits a v1 envelope"
STATUS_JSON_PRE="$(cd "$DVS_REPO" && dvs --json status)"
check "v1 status" "$(printf '%s' "$STATUS_JSON_PRE" | envelope_of)" "--json before the subcommand also emits a v1 envelope (L82)"

# get --json (delete local copy first so get does real work)
rm -f "$DVS_REPO/g.bin"
GET_JSON="$(cd "$DVS_REPO" && dvs get g.bin --json)"
check "v1 get" "$(printf '%s' "$GET_JSON" | envelope_of)" "get --json emits a v1 envelope"

# info, availability, history run against a local project too (L131-133, L189-192)
INFO_JSON="$(cd "$DVS_REPO" && dvs info --json)"
check "v1 info not_applicable" "$(printf '%s' "$INFO_JSON" | jq_py 'print("v%s %s %s" % (d.get("v"), d.get("command"), d.get("connection")))')" "info --json emits a v1 envelope, connection=not_applicable for a local project (L192)"
G_HASH="$(printf '%s' "$ADD_JSON" | jq_py 'print(d["files"][0]["hash"])')"
AVAIL_JSON="$(cd "$DVS_REPO" && dvs availability "$G_HASH" --json)"
check "v1 availability present" "$(printf '%s' "$AVAIL_JSON" | jq_py 'print("v%s %s %s" % (d.get("v"), d.get("command"), d["files"][0]["state"]))')" "availability --json emits a v1 envelope, the added hash is present (L131-132)"
HIST_JSON="$(cd "$DVS_REPO" && dvs history g.bin --json)"
check "v1 history 1" "$(printf '%s' "$HIST_JSON" | jq_py 'print("v%s %s %s" % (d.get("v"), d.get("command"), len(d["files"])))')" "history --json emits a v1 envelope with the one add of g.bin (L133)"
untestable "attach/login/logout --json need a dvs server"

say
say "=== CLAIM 2b: a --json argument error is an envelope with command cli, exit 2 (L89-90, L216) ==="
CLI_RC=0
CLI_JSON="$(dvs --json no-such-subcommand)" || CLI_RC=$?
check "v1 cli invalid_arguments" "$(printf '%s' "$CLI_JSON" | jq_py 'print("v%s %s %s" % (d.get("v"), d.get("command"), (d.get("error") or {}).get("code")))')" "unknown subcommand under --json is a cli envelope with invalid_arguments"
check "2" "$CLI_RC" "unknown subcommand under --json exits 2"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 3: project root discovery walks UP from nested subdir (L23-24) ==="
mkdir -p "$DVS_REPO/data/raw"
head -c 1024 /dev/urandom > "$DVS_REPO/data/raw/nested.bin"
(cd "$DVS_REPO" && dvs add data/raw/nested.bin >/dev/null)
# From data/raw (depth 2) run status; must find project root and list the file.
NESTED_RC=0
NESTED_OUT="$(cd "$DVS_REPO/data/raw" && dvs status --json)" || NESTED_RC=$?
# Paths are reported relative to project root (L128), so expect data/raw/nested.bin.
NESTED_OK="$(printf '%s' "$NESTED_OUT" | jq_py 'print("yes" if "data/raw/nested.bin" in [r["path"] for r in d["files"]] else "no")')"
check "yes" "$NESTED_OK" "status from data/raw/ resolves project root (rc=$NESTED_RC)"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 4: nested dvs.toml resolves to the NEAREST project (L24, L273) ==="
# Inside the existing repo create a nested project with its OWN dvs.toml + storage.
NESTED_PROJ="$DVS_REPO/inner"
NESTED_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_inner"
mkdir -p "$NESTED_PROJ" "$NESTED_STORE"
(cd "$NESTED_PROJ" && dvs init local "$NESTED_STORE" --root-dir "$(pwd)" >/dev/null)
# init nested below parent dvs.toml must succeed (local check, L273).
check "1" "$([ -f "$NESTED_PROJ/dvs.toml" ] && echo 1 || echo 0)" "nested init succeeds despite parent dvs.toml"
# Add a file from inside the nested project; its blob must land in NESTED_STORE,
# proving dvs resolved the nearest (inner) dvs.toml, not the outer one.
head -c 777 /dev/urandom > "$NESTED_PROJ/inner.bin"
INNER_HASH="$(cd "$NESTED_PROJ" && dvs add inner.bin --json | jq_py 'print(d["files"][0]["hash"])')"
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
say "=== CLAIM 5: gitignore entry format /<filename>, no duplicate on re-add (L791-792) ==="
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
say "=== CLAIM 6: no .git -> gitignore skipped, add still succeeds (L792-793) ==="
NOGIT_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_nogit_XXX)"
NOGIT_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_nogit"
mkdir "$NOGIT_STORE"
# Deliberately NO `git init` here.
(cd "$NOGIT_REPO" && dvs init local "$NOGIT_STORE" >/dev/null)
head -c 1024 /dev/urandom > "$NOGIT_REPO/b.bin"
NOGIT_RC=0
(cd "$NOGIT_REPO" && dvs add b.bin >/dev/null 2>&1) || NOGIT_RC=$?
check "0" "$NOGIT_RC" "add succeeds in repo with no .git"
# No .gitignore should have been written anywhere in the repo.
NOGIT_GI_COUNT="$(find "$NOGIT_REPO" -name .gitignore | wc -l | tr -d ' ')"
check "0" "$NOGIT_GI_COUNT" "no .gitignore written when no .git folder"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== CLAIM 7: parallelism DVS_NUM_THREADS honored (L778-782) ==="
# The actual thread count is not surfaced in plain CLI output, so the COUNT
# itself is UNTESTABLE via the CLI. What we CAN verify: setting the variable
# does not error and still produces correct results.
mkdir -p "$DVS_REPO/par"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p1.bin"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p2.bin"
head -c 1024 /dev/urandom > "$DVS_REPO/par/p3.bin"
PAR_RC=0
PAR_OUT="$(cd "$DVS_REPO" && DVS_NUM_THREADS=2 dvs add par/p1.bin par/p2.bin par/p3.bin --json)" || PAR_RC=$?
# Correct result: 3 copied outcomes.
PAR_COPIED="$(printf '%s' "$PAR_OUT" | jq_py 'print(sum(1 for r in d["files"] if r["outcome"] == "copied"))')"
check "0" "$PAR_RC" "DVS_NUM_THREADS=2 dvs add does not error"
check "3" "$PAR_COPIED" "DVS_NUM_THREADS=2 add produces correct results (3 copied)"
untestable "DVS_NUM_THREADS actual thread COUNT (L778-782) not observable via plain CLI output"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS_N pass, $FAIL_N fail ==="
say "(also $UNTESTABLE_N untestable)"

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
