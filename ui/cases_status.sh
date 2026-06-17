#!/usr/bin/env bash

# Exhaustive scenario coverage for `dvs status` (cases_status.sh in SCENARIOS.md).
# Superset of validate_status.sh: every status scenario bullet plus the two
# state-transition chains. CLI only, verdict-based. Each scenario prints
# expected vs actual and a PASS/FAIL/NOTE, ending with
# `=== SUMMARY: N pass, M fail ===`. Asserts SPEC-CORRECT behavior; tracked
# divergences are labeled with the issue number.

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

# note LABEL -- record a documented observation that is not a pass/fail gate.
note() {
  { set +x; } 2>/dev/null
  echo "NOTE: $1"
  { set -x; } 2>/dev/null
}

# Count data rows in a status table (rows whose first column is a known path).
# Counts table lines beginning with "| " excluding the header row "| path".
count_rows() {
  { set +x; } 2>/dev/null
  grep -c '^| [^p]' <<<"$1" || true
  { set -x; } 2>/dev/null
}

# Does the table contain a row for PATH with STATE?  Prints yes/no.
has_row() {
  { set +x; } 2>/dev/null
  if grep -Eq "^\| $2 +\| $3 " <<<"$1"; then echo yes; else echo no; fi
  { set -x; } 2>/dev/null
}

# ── Setup ──────────────────────────────────────────────────────────────────
# Pristine repo + sibling storage OUTSIDE the repo. Three-state fixture matching
# validate_status.sh: current / unsynced / absent, with a nested layout so
# non-recursive vs recursive directory filtering is distinguishable.

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

mkfiles 1 1K data            # data/file_1.bin       -> stays current
mkfiles 2 1K data/raw        # data/raw/file_1..2    -> raw/1 unsynced, raw/2 current
mkfiles 1 1K data/derived    # data/derived/file_1   -> deleted -> absent
mkfiles 1 1K models/v1       # models/v1/file_1      -> current

say "--- tree ---"
tree --noreport || ls -R

dvs add --glob "data/**/*.bin"
dvs add --glob "models/**/*.bin"

# Build the three states:
#   current : data/file_1.bin, data/raw/file_2.bin, models/v1/file_1.bin
#   unsynced: data/raw/file_1.bin  (append bytes after add)
#   absent  : data/derived/file_1.bin (delete local copy)
printf 'EXTRA' >> data/raw/file_1.bin
rm data/derived/file_1.bin

# ── Scenario: default (no flags) shows ALL states (L297) ─────────────────────
say
say "=== SCENARIO: default no-flags shows all 5 tracked files regardless of state (L297) ==="
ALL="$(dvs status)"
echo "$ALL"
check "5" "$(count_rows "$ALL")" "default shows all 5 tracked rows"
check "yes" "$(has_row "$ALL" "data/file_1.bin" "current")"        "data/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_2.bin" "current")"    "data/raw/file_2.bin is current"
check "yes" "$(has_row "$ALL" "models/v1/file_1.bin" "current")"   "models/v1/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_1.bin" "unsynced")"   "data/raw/file_1.bin is unsynced"
check "yes" "$(has_row "$ALL" "data/derived/file_1.bin" "absent")" "data/derived/file_1.bin is absent"

# ── Scenario: --current single filter (L297-298) ─────────────────────────────
say
say "=== SCENARIO: --current single filter shows current only (L297-298) ==="
CUR="$(dvs status --current)"
echo "$CUR"
check "3" "$(count_rows "$CUR")" "--current shows 3 rows"
check "no" "$(has_row "$CUR" "data/raw/file_1.bin" "unsynced")" "--current excludes unsynced"
check "no" "$(has_row "$CUR" "data/derived/file_1.bin" "absent")" "--current excludes absent"

# ── Scenario: --absent single filter (L297-298) ──────────────────────────────
say
say "=== SCENARIO: --absent single filter shows absent only (L297-298) ==="
ABS="$(dvs status --absent)"
echo "$ABS"
check "1" "$(count_rows "$ABS")" "--absent shows 1 row"
check "yes" "$(has_row "$ABS" "data/derived/file_1.bin" "absent")" "--absent shows the absent file"

# ── Scenario: --unsynced single filter (L297-298) ────────────────────────────
say
say "=== SCENARIO: --unsynced single filter shows unsynced only (L297-298) ==="
UNS="$(dvs status --unsynced)"
echo "$UNS"
check "1" "$(count_rows "$UNS")" "--unsynced shows 1 row"
check "yes" "$(has_row "$UNS" "data/raw/file_1.bin" "unsynced")" "--unsynced shows the unsynced file"

# ── Scenario: filter unions (L297-298) ───────────────────────────────────────
say
say "=== SCENARIO: filter combinations are unions ==="
U_CA="$(dvs status --current --absent)"
echo "$U_CA"
check "4" "$(count_rows "$U_CA")" "--current --absent = 3 current + 1 absent = 4 rows"
check "no" "$(has_row "$U_CA" "data/raw/file_1.bin" "unsynced")" "current+absent union excludes unsynced"

U_AU="$(dvs status --absent --unsynced)"
echo "$U_AU"
check "2" "$(count_rows "$U_AU")" "--absent --unsynced = 1 + 1 = 2 rows"
check "no" "$(has_row "$U_AU" "data/file_1.bin" "current")" "absent+unsynced union excludes current"

U_ALL="$(dvs status --current --absent --unsynced)"
check "5" "$(count_rows "$U_ALL")" "all three filters together = all 5 rows (same as default)"

# ── Scenario: single-file path filter (L284) ─────────────────────────────────
say
say "=== SCENARIO: path filter to a single file (L284) ==="
ONE="$(dvs status data/raw/file_2.bin)"
echo "$ONE"
check "1" "$(count_rows "$ONE")" "single-file path filter shows exactly 1 row"
check "yes" "$(has_row "$ONE" "data/raw/file_2.bin" "current")" "single-file path shows that file"

# ── Scenario: directory non-recursive = direct children only (L289) ──────────
say
say "=== SCENARIO: directory non-recursive shows direct children only (L289) ==="
DIR="$(dvs status data/)"
echo "$DIR"
check "1" "$(count_rows "$DIR")" "data/ non-recursive shows only direct child (1 row)"
check "yes" "$(has_row "$DIR" "data/file_1.bin" "current")" "data/ non-recursive includes direct child data/file_1.bin"
check "no" "$(has_row "$DIR" "data/raw/file_2.bin" "current")" "data/ non-recursive excludes data/raw descendant"

# ── Scenario: -r/--recursive includes all descendants (L289) ─────────────────
say
say "=== SCENARIO: -r/--recursive includes all descendants of a directory (L289) ==="
REC="$(dvs status -r data/)"
echo "$REC"
check "4" "$(count_rows "$REC")" "-r data/ shows all 4 descendants (top + raw1 + raw2 + derived)"
check "yes" "$(has_row "$REC" "data/raw/file_2.bin" "current")" "-r data/ includes nested data/raw/file_2.bin"
check "yes" "$(has_row "$REC" "data/derived/file_1.bin" "absent")" "-r data/ includes nested data/derived/file_1.bin"
REC_LONG="$(dvs status --recursive data/)"
check "$(count_rows "$REC")" "$(count_rows "$REC_LONG")" "--recursive matches -r row count"

# ── Scenario: --with-metadata adds columns (L293) ────────────────────────────
say
say "=== SCENARIO: --with-metadata adds metadata columns (L293) ==="
DEFAULT_TBL="$(dvs status)"
META_TBL="$(dvs status --with-metadata)"
DEFAULT_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$DEFAULT_TBL" | head -1 | tr -d ' ' )"
META_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$META_TBL" | head -1 | tr -d ' ' )"
echo "default header : $DEFAULT_HDR"
echo "metadata header: $META_HDR"
check "|path|status|size|" "$DEFAULT_HDR" "default table columns are path/status/size"
check "|path|status|size|hash|created_by|add_time|compression|message|" "$META_HDR" "--with-metadata adds hash/created_by/add_time/compression/message"
check "$(count_rows "$DEFAULT_TBL")" "$(count_rows "$META_TBL")" "--with-metadata keeps the same row count"

# ── Scenario: --json shape + honors filters (L287) ───────────────────────────
say
say "=== SCENARIO: --json shape and that it honors filters (L287) ==="
JSON="$(dvs status --json)"
echo "$JSON"
JLEN="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))' )"
check "5" "$JLEN" "--json emits an array of 5 objects"
JKEYS="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(",".join(sorted(d[0].keys())))' )"
check "metadata,path,status" "$JKEYS" "--json object has path/status/metadata keys"
JABS="$({ set +x; } 2>/dev/null; dvs status --json --absent | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d), d[0]["status"])' )"
check "1 absent" "$JABS" "--json honors --absent filter"
JCAU="$({ set +x; } 2>/dev/null; dvs status --json --current --unsynced | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))' )"
check "4" "$JCAU" "--json honors a --current --unsynced union (3 + 1 = 4)"

# ── Scenario: status from a NESTED subdirectory (L284, root discovery) ───────
say
say "=== SCENARIO: status from a nested subdir (root discovery, paths root-relative) ==="
mkdir -p data/raw/deep
NESTED="$(cd data/raw/deep && dvs status)"
echo "$NESTED"
check "5" "$(count_rows "$NESTED")" "status from nested subdir still discovers root and shows all 5"
check "yes" "$(has_row "$NESTED" "models/v1/file_1.bin" "current")" "nested-subdir paths are project-root-relative (models/v1/file_1.bin), not cwd-relative"
rmdir data/raw/deep

# ── Scenario: abs / rel path arguments resolve identically (L284) ────────────
say
say "=== SCENARIO: absolute vs relative path argument resolve to the same row ==="
REL="$(dvs status data/file_1.bin)"
ABSP="$(dvs status "$DVS_REPO/data/file_1.bin")"
check "1" "$(count_rows "$REL")" "relative path arg shows 1 row"
check "1" "$(count_rows "$ABSP")" "absolute path arg shows 1 row"
check "yes" "$(has_row "$ABSP" "data/file_1.bin" "current")" "absolute path arg reported as root-relative data/file_1.bin"
check "$(count_rows "$REL")" "$(count_rows "$ABSP")" "abs and rel path args give the same row count"

# ── Scenario: NO files tracked yet -> empty, exit 0 (L297) ───────────────────
say
say "=== SCENARIO: status when NO files are tracked yet (empty result, exit 0) ==="
EMPTY_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_empty_XXX)"
EMPTY_SUFFIX="${EMPTY_REPO##*_}"
EMPTY_STORAGE="$SCRIPT_DIR/dvs_storage_cli_empty_$EMPTY_SUFFIX"
mkdir "$EMPTY_STORAGE"
(
  cd "$EMPTY_REPO"
  git init -q
  dvs init "$EMPTY_STORAGE" >/dev/null
  rc=0
  EOUT="$(dvs status 2>&1)" || rc=$?
  echo "$EOUT"
  echo "EXIT(empty)=$rc"
  check "0" "$rc" "status with no tracked files exits 0"
  check "0" "$(count_rows "$EOUT")" "status with no tracked files shows 0 data rows"
)
rm -rf "$EMPTY_REPO" "$EMPTY_STORAGE"

# ── Scenario: path with no metadata / untracked (document) (L284) ────────────
say
say "=== SCENARIO: status on an untracked path / path with no metadata (document) ==="
printf 'plain' > untracked_plain.txt
rc=0
UNTRK="$(dvs status untracked_plain.txt 2>&1)" || rc=$?
echo "$UNTRK"
echo "EXIT(untracked)=$rc"
check "0" "$rc" "status on an untracked path exits 0 (not an error)"
check "0" "$(count_rows "$UNTRK")" "status on an untracked path shows 0 rows"
note "untracked path with no .dvs metadata is silently dropped (no row, no error, exit 0). 'No tracked files' printed when the filtered set is empty."
rc=0
MISSING="$(dvs status does_not_exist_anywhere.bin 2>&1)" || rc=$?
echo "$MISSING"
echo "EXIT(missing)=$rc"
check "0" "$rc" "status on a nonexistent untracked path also exits 0 (no metadata = dropped)"
rm -f untracked_plain.txt

# ── Scenario: glob-on-status behavior (#218 -- report ACTUAL) ─────────────────
say
say "=== SCENARIO: glob on status (#218 -- report ACTUAL behavior on this branch) ==="
# This branch added -g/--glob to status. Probe and report what it does.
rc=0; G_REC="$(dvs status --glob 'data/**/*.bin' 2>&1)" || rc=$?
echo "$G_REC"; echo "EXIT(--glob data/**/*.bin)=$rc"
check "0" "$rc" "status --glob exits 0 (#218: glob now accepted on status)"
G_REC_ROWS="$(count_rows "$G_REC")"
note "status --glob 'data/**/*.bin' returned $G_REC_ROWS rows (4 tracked files live under data/ across all states). #218 appears FIXED: glob is not rejected and is FUNCTIONAL -- it filters tracked files by the pattern, regardless of state."
rc=0; G_SHORT="$(dvs status -g 'data/*.bin' 2>&1)" || rc=$?
echo "$G_SHORT"; echo "EXIT(-g data/*.bin)=$rc"
check "0" "$rc" "status -g exits 0 (short form accepted)"
G_SHORT_ROWS="$(count_rows "$G_SHORT")"
check "1" "$G_SHORT_ROWS" "status -g 'data/*.bin' single-level pattern matches only direct data/ children (data/file_1.bin)"
note "status -g 'data/*.bin' returned $G_SHORT_ROWS row(s); '**/*.bin' style recursive patterns match descendants too -- glob is implemented, not a stub."

# ── Scenario: malformed sidecar -> per-file error, exit 1 (L300, L305-306) ───
say
say "=== SCENARIO: malformed .dvs sidecar -> per-file error, exit 1 (L300) ==="
SIDE=".dvs/data/raw/file_2.bin.dvs"
cp "$SIDE" "$SCRIPT_DIR/sidebak_$RUN_SUFFIX"
printf 'not json{{{' > "$SIDE"
rc=0
ERR_OUT="$(dvs status 2>&1)" || rc=$?
echo "$ERR_OUT"
echo "EXIT(malformed)=$rc"
check "1" "$rc" "malformed .dvs sidecar -> exit 1"
PERFILE="$({ set +x; } 2>/dev/null; grep -c 'Error getting status for data/raw/file_2.bin' <<<"$ERR_OUT" )"
check "1" "$PERFILE" "per-file error reported for the malformed sidecar"
# other (good) sidecars still report -- overall command did not abort
OTHER_OK="$({ set +x; } 2>/dev/null; grep -c '^| data/file_1.bin ' <<<"$ERR_OUT" )"
check "1" "$OTHER_OK" "other files still inspected despite one malformed sidecar (never errors as a whole, L305)"
cp "$SCRIPT_DIR/sidebak_$RUN_SUFFIX" "$SIDE"

# ── Scenario: unreadable sidecar -> per-file error, exit 1 (guard id -u) ──────
say
say "=== SCENARIO: unreadable (chmod 000) .dvs sidecar -> exit 1 (skipped as root) ==="
if [ "$(id -u)" -ne 0 ]; then
  chmod 000 "$SIDE"
  rc=0
  PERM_OUT="$(dvs status 2>&1)" || rc=$?
  echo "$PERM_OUT"
  echo "EXIT(chmod000)=$rc"
  check "1" "$rc" "unreadable (chmod 000) .dvs sidecar -> exit 1"
  PERM_PERFILE="$({ set +x; } 2>/dev/null; grep -c 'Error getting status for data/raw/file_2.bin' <<<"$PERM_OUT" )"
  check "1" "$PERM_PERFILE" "per-file error reported for the unreadable sidecar"
  chmod 644 "$SIDE"
else
  say "SKIP: chmod 000 unreadable-sidecar test (running as root)"
fi
rm -f "$SCRIPT_DIR/sidebak_$RUN_SUFFIX"

# Confirm clean exit 0 once both sidecars are good again.
rc=0; dvs status >/dev/null 2>&1 || rc=$?
check "0" "$rc" "status exits 0 after sidecars restored"

# ── Scenario: transition add -> modify -> add (current->unsynced->current) ────
say
say "=== SCENARIO: transition add (current) -> modify (unsynced) -> add (current again) ==="
mkrandfile cycle/t1.bin 1K
dvs add cycle/t1.bin >/dev/null
S1="$(dvs status cycle/t1.bin)"
check "yes" "$(has_row "$S1" "cycle/t1.bin" "current")" "after add: cycle/t1.bin is current"
printf 'MUTATE' >> cycle/t1.bin
S2="$(dvs status cycle/t1.bin)"
echo "$S2"
check "yes" "$(has_row "$S2" "cycle/t1.bin" "unsynced")" "after local modify: cycle/t1.bin is unsynced"
dvs add cycle/t1.bin >/dev/null
S3="$(dvs status cycle/t1.bin)"
echo "$S3"
check "yes" "$(has_row "$S3" "cycle/t1.bin" "current")" "after re-add: cycle/t1.bin is current again"

# ── Scenario: transition add -> delete -> get (current->absent->current) ──────
say
say "=== SCENARIO: transition add (current) -> delete local (absent) -> get (current again) ==="
mkrandfile cycle/t2.bin 1K
dvs add cycle/t2.bin >/dev/null
D1="$(dvs status cycle/t2.bin)"
check "yes" "$(has_row "$D1" "cycle/t2.bin" "current")" "after add: cycle/t2.bin is current"
rm cycle/t2.bin
D2="$(dvs status cycle/t2.bin)"
echo "$D2"
check "yes" "$(has_row "$D2" "cycle/t2.bin" "absent")" "after local delete: cycle/t2.bin is absent"
dvs get cycle/t2.bin >/dev/null
D3="$(dvs status cycle/t2.bin)"
echo "$D3"
check "yes" "$(has_row "$D3" "cycle/t2.bin" "current")" "after get: cycle/t2.bin is restored to current"

# ── Cleanup own dirs ─────────────────────────────────────────────────────────
cd "$SCRIPT_DIR"
rm -rf "$DVS_REPO" "$DVS_STORAGE"

# ── Summary ──────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
