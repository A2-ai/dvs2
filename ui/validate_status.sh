#!/usr/bin/env bash

# Validate CLI-observable `dvs status` claims from specs.md (Script 4 in ui/TODO.md).
# CLI only, verdict-based. Each claim prints expected vs actual and a PASS/FAIL,
# ending with `=== SUMMARY: N pass, M fail ===`.
# Claims come from specs.md, status, unless a claim names another section.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

PASS=0
FAIL=0

# check EXPECTED ACTUAL LABEL
check() {
  { set +x; } 2>/dev/null
  if [ "$1" = "$2" ]; then
    echo "PASS: $3"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $3 (want=[$1] got=[$2])"
    FAIL=$((FAIL + 1))
  fi
  { set -x; } 2>/dev/null
}

# Run a python expression over the --json envelope on stdin. Prints
# <parse-error> instead of aborting, so the script always reaches SUMMARY.
jq_py() { python3 -c "import sys,json;d=json.load(sys.stdin);$1" 2>/dev/null || echo "<parse-error>"; }

# The row and filter claims read `status --json`. The human table is display
# text, so only its paths and its summary count are checked.

# Number of rows in a status --json envelope.
count_rows() {
  { set +x; } 2>/dev/null
  printf '%s' "$1" | jq_py 'print(len(d["files"]))'
  { set -x; } 2>/dev/null
}

# Does a status --json envelope have a row for PATH with STATE? Prints yes/no.
has_row() {
  { set +x; } 2>/dev/null
  printf '%s' "$1" | jq_py "print('yes' if any(f['path'] == '$2' and f['status'] == '$3' for f in d['files']) else 'no')"
  { set -x; } 2>/dev/null
}

# The rows of a status --json envelope, one "path status" line each.
show_rows() {
  { set +x; } 2>/dev/null
  printf '%s' "$1" | jq_py 'print("\n".join("%s %s" % (f["path"], f["status"]) for f in d["files"]))'
  { set -x; } 2>/dev/null
}

# How many of the paths after the first argument a human output names.
named_paths() {
  { set +x; } 2>/dev/null
  local out="$1" n=0 p
  shift
  for p in "$@"; do grep -qFw -- "$p" <<<"$out" && n=$((n + 1)); done
  echo "$n"
  { set -x; } 2>/dev/null
}

# The first number on the last line of a human output (its summary line).
summary_count() {
  { set +x; } 2>/dev/null
  printf '%s\n' "$1" | tail -1 | grep -oE '[0-9]+' | head -1 || true
  { set -x; } 2>/dev/null
}

# Which of the words after the first argument a human output carries, as words.
words_in() {
  { set +x; } 2>/dev/null
  local out="$1" found="" w
  shift
  for w in "$@"; do grep -qw -- "$w" <<<"$out" && found="$found $w"; done
  echo "${found# }"
  { set -x; } 2>/dev/null
}

# ── Setup ──────────────────────────────────────────────────────────────────

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init local "$DVS_STORAGE"

# Nested layout with a direct child of data/ so non-recursive vs recursive
# directory filtering is distinguishable.
mkfiles 1 1K data            # data/file_1.bin       -> stays current
mkfiles 2 1K data/raw        # data/raw/file_1..2    -> raw/1 modified, raw/2 current
mkfiles 1 1K data/derived    # data/derived/file_1   -> deleted -> absent
mkfiles 1 1K models/v1       # models/v1/file_1      -> current
mkrandfile other/stale.bin 300     # -> stale
mkrandfile other/donor.bin 400     # source of newer metadata, untracked again below
mkrandfile other/unsynced.bin 350  # -> unsynced, never added here

say "--- tree ---"
tree --noreport || ls -R

dvs add --glob "data/**/*.bin"
dvs add --glob "models/**/*.bin"
dvs add other/stale.bin other/donor.bin

# Build the five states:
#   current : data/file_1.bin, data/raw/file_2.bin, models/v1/file_1.bin
#   modified: data/raw/file_1.bin  (append bytes after add: differs from the
#             metadata and from the baseline add recorded)
#   absent  : data/derived/file_1.bin (delete local copy)
#   stale   : other/stale.bin  (metadata advanced, as after a git pull, while
#             the local file still matches its baseline)
#   unsynced: other/unsynced.bin (metadata arrives for a path dvs never wrote
#             or added in this clone, so there is no baseline, specs.md, Hash cache)
printf 'EXTRA' >> data/raw/file_1.bin
rm data/derived/file_1.bin
cp .dvs/other/donor.bin.dvs .dvs/other/stale.bin.dvs
cp .dvs/other/donor.bin.dvs .dvs/other/unsynced.bin.dvs
rm .dvs/other/donor.bin.dvs other/donor.bin

# ── Claim: default (no flags) shows ALL tracked files ─────────────────────────
say
say "=== CLAIM: default no-flags shows all 7 tracked files regardless of state ==="
ALL_HR="$(dvs status)"
echo "$ALL_HR"
TRACKED="data/file_1.bin data/raw/file_1.bin data/raw/file_2.bin data/derived/file_1.bin models/v1/file_1.bin other/stale.bin other/unsynced.bin"
# shellcheck disable=SC2086
check "7" "$(named_paths "$ALL_HR" $TRACKED)" "default human output names all 7 tracked files"
# Human output ends with a summary line. Check its count, not its wording.
check "7" "$(summary_count "$ALL_HR")" "the human summary line counts 7 files"
ALL="$(dvs status --json)"

# ── Claim: five states reported correctly ─────────────────────────────────────
say
say "=== CLAIM: states current/absent/stale/modified/unsynced reported correctly (specs.md, High-level overview) ==="
check "yes" "$(has_row "$ALL" "data/file_1.bin" "current")"           "data/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_2.bin" "current")"       "data/raw/file_2.bin is current"
check "yes" "$(has_row "$ALL" "models/v1/file_1.bin" "current")"      "models/v1/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_1.bin" "modified")"      "data/raw/file_1.bin (edited after add) is modified"
check "yes" "$(has_row "$ALL" "data/derived/file_1.bin" "absent")"    "data/derived/file_1.bin is absent"
check "yes" "$(has_row "$ALL" "other/stale.bin" "stale")"             "other/stale.bin (metadata advanced, local = baseline) is stale"
check "yes" "$(has_row "$ALL" "other/unsynced.bin" "unsynced")"       "other/unsynced.bin (differs, no baseline) is unsynced"

# ── Claim: --current filters to current only ──────────────────────────────────
say
say "=== CLAIM: --current filters to current only ==="
CUR="$(dvs status --current --json)"
show_rows "$CUR"
check "3" "$(count_rows "$CUR")" "--current shows 3 rows"
check "no" "$(has_row "$CUR" "data/raw/file_1.bin" "modified")" "--current excludes modified"
check "no" "$(has_row "$CUR" "data/derived/file_1.bin" "absent")" "--current excludes absent"

# ── Claim: --absent filters to absent only ────────────────────────────────────
say
say "=== CLAIM: --absent filters to absent only ==="
ABS="$(dvs status --absent --json)"
show_rows "$ABS"
check "1" "$(count_rows "$ABS")" "--absent shows 1 row"
check "yes" "$(has_row "$ABS" "data/derived/file_1.bin" "absent")" "--absent shows the absent file"

# ── Claim: --stale, --modified, --unsynced filter to their state ──────────────
say
say "=== CLAIM: --stale filters to stale only ==="
STA="$(dvs status --stale --json)"
show_rows "$STA"
check "1" "$(count_rows "$STA")" "--stale shows 1 row"
check "yes" "$(has_row "$STA" "other/stale.bin" "stale")" "--stale shows the stale file"

say
say "=== CLAIM: --modified filters to modified only ==="
MOD="$(dvs status --modified --json)"
show_rows "$MOD"
check "1" "$(count_rows "$MOD")" "--modified shows 1 row"
check "yes" "$(has_row "$MOD" "data/raw/file_1.bin" "modified")" "--modified shows the modified file"

say
say "=== CLAIM: --unsynced filters to unsynced only ==="
UNS="$(dvs status --unsynced --json)"
show_rows "$UNS"
check "1" "$(count_rows "$UNS")" "--unsynced shows 1 row"
check "yes" "$(has_row "$UNS" "other/unsynced.bin" "unsynced")" "--unsynced shows the unsynced file"

# ── Claim: multiple filters combine as a union ────────────────────────────────
say
say "=== CLAIM: --current --absent combine (union) ==="
UNION="$(dvs status --current --absent --json)"
show_rows "$UNION"
check "4" "$(count_rows "$UNION")" "--current --absent shows 3 current + 1 absent = 4 rows"
check "no" "$(has_row "$UNION" "data/raw/file_1.bin" "modified")" "union excludes modified"

# ── Claim: path filter to a single file ───────────────────────────────────────
say
say "=== CLAIM: path filter to a single file ==="
ONE="$(dvs status --json data/raw/file_2.bin)"
show_rows "$ONE"
check "1" "$(count_rows "$ONE")" "single-file path filter shows exactly 1 row"
check "yes" "$(has_row "$ONE" "data/raw/file_2.bin" "current")" "single-file path shows that file"

# ── Claim: a path matching no tracked file is dropped silently ────────────────
say
say "=== CLAIM: a path matching no tracked file is dropped silently, exit 0 ==="
rc=0; BOGUS="$(dvs status --json data/raw/file_2.bin bogus.bin)" || rc=$?
show_rows "$BOGUS"
check "0" "$rc" "'status good bogus' exits 0"
check "1" "$(count_rows "$BOGUS")" "'status good bogus' shows only the matching path"

# ── Claim: directory path, non-recursive = direct children only ───────────────
say
say "=== CLAIM: directory non-recursive = direct children only ==="
DIR="$(dvs status --json data/)"
show_rows "$DIR"
check "1" "$(count_rows "$DIR")" "data/ non-recursive shows only direct child (1 row)"
check "yes" "$(has_row "$DIR" "data/file_1.bin" "current")" "data/ non-recursive includes direct child data/file_1.bin"
check "no" "$(has_row "$DIR" "data/raw/file_2.bin" "current")" "data/ non-recursive excludes data/raw descendant"

# ── Claim: -r/--recursive includes all descendants ────────────────────────────
say
say "=== CLAIM: -r/--recursive includes all descendants of a directory ==="
REC="$(dvs status --json -r data/)"
show_rows "$REC"
check "4" "$(count_rows "$REC")" "-r data/ shows all 4 descendants (top + raw1 + raw2 + derived)"
check "yes" "$(has_row "$REC" "data/raw/file_2.bin" "current")" "-r data/ includes nested data/raw/file_2.bin"
check "yes" "$(has_row "$REC" "data/derived/file_1.bin" "absent")" "-r data/ includes nested data/derived/file_1.bin"
# long form parity
REC_LONG="$(dvs status --json --recursive data/)"
check "$(count_rows "$REC")" "$(count_rows "$REC_LONG")" "--recursive matches -r row count"

# ── Claim: --with-metadata adds metadata columns ──────────────────────────────
say
say "=== CLAIM: --with-metadata shows all metadata columns vs default ==="
# The metadata column names as words, in any order and at any width.
META_COLS="hash created_by add_time compression message"
DEFAULT_TBL="$(dvs status)"
META_TBL="$(dvs status --with-metadata)"
echo "$META_TBL"
# shellcheck disable=SC2086
check "" "$(words_in "$DEFAULT_TBL" $META_COLS)" "default table has no metadata columns"
# shellcheck disable=SC2086
check "$META_COLS" "$(words_in "$META_TBL" $META_COLS)" "--with-metadata adds hash, created_by, add_time, compression, message"

# ── Claim: --json output shape ────────────────────────────────────────────────
say
say "=== CLAIM: --json output shape (specs.md, Machine output) ==="
JSON="$(dvs status --json)"
echo "$JSON"
JENV="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'print(d["v"], d["command"], d["outcome"], d["error"], d["counts"]["total"], d["counts"]["succeeded"], d["counts"]["failed"])' )"
check "1 status complete None 7 7 0" "$JENV" "--json is a v1 envelope: command=status, outcome=complete, counts total=succeeded=7"
JLEN="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'print(len(d["files"]))' )"
check "7" "$JLEN" "--json files has 7 rows"
# Rows have fixed keys. Extra keys are tolerated.
JKEYS="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'need={"path","status","metadata","working","baseline","code","message","hint"};print(",".join(sorted(need-set(d["files"][0]))) or "ok")' )"
check "ok" "$JKEYS" "--json row has path/status/metadata/working/baseline/code/message/hint keys"
# Row facts behind the states: absent -> working.state absent,
# stale -> local matches the baseline, not the metadata, unsynced -> no baseline.
JFACTS="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py '
r={f["path"]:f for f in d["files"]}
a=r["data/derived/file_1.bin"]; s=r["other/stale.bin"]; u=r["other/unsynced.bin"]
w=lambda f:(f["working"]["hashes"] or {}).get("blake3")
print(a["working"]["state"],
      w(s)==s["baseline"]["blake3"]!=s["metadata"]["hashes"]["blake3"],
      u["baseline"])' )"
check "absent True None" "$JFACTS" "row facts: absent working.state=absent, stale working=baseline!=metadata, unsynced baseline=null"
# json honors filters
JABS="$({ set +x; } 2>/dev/null; dvs status --json --absent | jq_py 'print(len(d["files"]), d["files"][0]["status"])' )"
check "1 absent" "$JABS" "--json honors --absent filter"

# ── Claim: exit 1 if a file cannot be inspected ───────────────────────────────
say
say "=== CLAIM: exit 1 if a file cannot be inspected ==="
SIDE=".dvs/data/raw/file_2.bin.dvs"
cp "$SIDE" "$SCRIPT_DIR/sidebak_$RUN_SUFFIX"

# (a) malformed JSON sidecar -> deterministic, root-safe.
printf 'not json{{{' > "$SIDE"
rc=0
ERR_OUT="$(dvs status 2>&1)" || rc=$?
echo "$ERR_OUT"
echo "EXIT(malformed)=$rc"
check "1" "$rc" "malformed .dvs sidecar -> exit 1"
PERFILE="$({ set +x; } 2>/dev/null; grep -qF 'data/raw/file_2.bin' <<<"$ERR_OUT" && echo yes || echo no)"
check "yes" "$PERFILE" "per-file error names data/raw/file_2.bin, the broken sidecar"
# The same failure as a --json row: status=error and code bad_metadata.
rc=0; ERR_JSON="$(dvs status --json 2>/dev/null)" || rc=$?
ERR_ROW="$({ set +x; } 2>/dev/null; printf '%s' "$ERR_JSON" | jq_py 'r=[f for f in d["files"] if f["path"]=="data/raw/file_2.bin"][0];print(d["outcome"], d["counts"]["failed"], r["status"], r["code"])' )"
check "partial 1 error bad_metadata" "$ERR_ROW" "--json: outcome=partial, counts.failed=1, row status=error, code=bad_metadata"
cp "$SCRIPT_DIR/sidebak_$RUN_SUFFIX" "$SIDE"

# (b) unreadable sidecar (chmod 000). Skipped when running as root (would still read).
if [ "$(id -u)" -ne 0 ]; then
  chmod 000 "$SIDE"
  rc=0
  PERM_OUT="$(dvs status 2>&1)" || rc=$?
  echo "$PERM_OUT"
  echo "EXIT(chmod000)=$rc"
  check "1" "$rc" "unreadable (chmod 000) .dvs sidecar -> exit 1"
  chmod 644 "$SIDE"
else
  say "SKIP: chmod 000 unreadable-sidecar test (running as root)"
fi
rm -f "$SCRIPT_DIR/sidebak_$RUN_SUFFIX"

# Restore: confirm clean exit 0 once sidecar is good again.
rc=0; dvs status >/dev/null 2>&1 || rc=$?
check "0" "$rc" "status exits 0 after sidecar restored"

# ── Summary ──────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
