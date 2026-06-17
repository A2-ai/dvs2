#!/usr/bin/env bash

# Validate CLI-observable `dvs status` claims from specs.md (Script 4 in ui/TODO.md).
# CLI only, verdict-based. Each claim prints expected vs actual and a PASS/FAIL,
# ending with `=== SUMMARY: N pass, M fail ===`.

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

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# Nested layout with a direct child of data/ so non-recursive vs recursive
# directory filtering is distinguishable.
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

# ── Claim: status --help matches spec block (L260-276) ───────────────────────
say
say "=== CLAIM: status --help matches spec (option order: json, threads, recursive, current, absent, unsynced, with-metadata) ==="
HELP="$(dvs status --help)"
# Option flags in the order they appear in --help.
OPT_ORDER="$({ set +x; } 2>/dev/null; grep -oE '\-\-(json|threads|recursive|current|absent|unsynced|with-metadata)' <<<"$HELP" | tr '\n' ',' )"
check "--json,--threads,--recursive,--current,--absent,--unsynced,--with-metadata," "$OPT_ORDER" "help option order matches spec"
USAGE="$({ set +x; } 2>/dev/null; grep -oE 'Usage: dvs status \[OPTIONS\] \[PATHS\]\.\.\.' <<<"$HELP" )"
check "Usage: dvs status [OPTIONS] [PATHS]..." "$USAGE" "help usage line matches spec"
SHORTR="$({ set +x; } 2>/dev/null; grep -c '\-r, --recursive' <<<"$HELP" )"
check "1" "$SHORTR" "help documents short -r for --recursive"

# ── Claim: default (no flags) shows ALL tracked files (L279) ─────────────────
say
say "=== CLAIM: default no-flags shows all 5 tracked files regardless of state (L279) ==="
ALL="$(dvs status)"
echo "$ALL"
check "5" "$(count_rows "$ALL")" "default shows all 5 tracked rows"

# ── Claim: three states reported correctly (L47-49) ──────────────────────────
say
say "=== CLAIM: states current/absent/unsynced reported correctly (L47-49) ==="
check "yes" "$(has_row "$ALL" "data/file_1.bin" "current")"           "data/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_2.bin" "current")"       "data/raw/file_2.bin is current"
check "yes" "$(has_row "$ALL" "models/v1/file_1.bin" "current")"      "models/v1/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_1.bin" "unsynced")"      "data/raw/file_1.bin is unsynced"
check "yes" "$(has_row "$ALL" "data/derived/file_1.bin" "absent")"    "data/derived/file_1.bin is absent"

# ── Claim: --current filters to current only (L272, L279) ────────────────────
say
say "=== CLAIM: --current filters to current only (L272) ==="
CUR="$(dvs status --current)"
echo "$CUR"
check "3" "$(count_rows "$CUR")" "--current shows 3 rows"
check "no" "$(has_row "$CUR" "data/raw/file_1.bin" "unsynced")" "--current excludes unsynced"
check "no" "$(has_row "$CUR" "data/derived/file_1.bin" "absent")" "--current excludes absent"

# ── Claim: --absent filters to absent only (L273) ────────────────────────────
say
say "=== CLAIM: --absent filters to absent only (L273) ==="
ABS="$(dvs status --absent)"
echo "$ABS"
check "1" "$(count_rows "$ABS")" "--absent shows 1 row"
check "yes" "$(has_row "$ABS" "data/derived/file_1.bin" "absent")" "--absent shows the absent file"

# ── Claim: --unsynced filters to unsynced only (L274) ────────────────────────
say
say "=== CLAIM: --unsynced filters to unsynced only (L274) ==="
UNS="$(dvs status --unsynced)"
echo "$UNS"
check "1" "$(count_rows "$UNS")" "--unsynced shows 1 row"
check "yes" "$(has_row "$UNS" "data/raw/file_1.bin" "unsynced")" "--unsynced shows the unsynced file"

# ── Claim: multiple filters combine as a union (L279-280) ────────────────────
say
say "=== CLAIM: --current --absent combine (union) (L279-280) ==="
UNION="$(dvs status --current --absent)"
echo "$UNION"
check "4" "$(count_rows "$UNION")" "--current --absent shows 3 current + 1 absent = 4 rows"
check "no" "$(has_row "$UNION" "data/raw/file_1.bin" "unsynced")" "union excludes unsynced"

# ── Claim: path filter to a single file (L266) ───────────────────────────────
say
say "=== CLAIM: path filter to a single file (L266) ==="
ONE="$(dvs status data/raw/file_2.bin)"
echo "$ONE"
check "1" "$(count_rows "$ONE")" "single-file path filter shows exactly 1 row"
check "yes" "$(has_row "$ONE" "data/raw/file_2.bin" "current")" "single-file path shows that file"

# ── Claim: directory path, non-recursive = direct children only (L271) ───────
say
say "=== CLAIM: directory non-recursive = direct children only (L271) ==="
DIR="$(dvs status data/)"
echo "$DIR"
check "1" "$(count_rows "$DIR")" "data/ non-recursive shows only direct child (1 row)"
check "yes" "$(has_row "$DIR" "data/file_1.bin" "current")" "data/ non-recursive includes direct child data/file_1.bin"
check "no" "$(has_row "$DIR" "data/raw/file_2.bin" "current")" "data/ non-recursive excludes data/raw descendant"

# ── Claim: -r/--recursive includes all descendants (L271) ────────────────────
say
say "=== CLAIM: -r/--recursive includes all descendants of a directory (L271) ==="
REC="$(dvs status -r data/)"
echo "$REC"
check "4" "$(count_rows "$REC")" "-r data/ shows all 4 descendants (top + raw1 + raw2 + derived)"
check "yes" "$(has_row "$REC" "data/raw/file_2.bin" "current")" "-r data/ includes nested data/raw/file_2.bin"
check "yes" "$(has_row "$REC" "data/derived/file_1.bin" "absent")" "-r data/ includes nested data/derived/file_1.bin"
# long form parity
REC_LONG="$(dvs status --recursive data/)"
check "$(count_rows "$REC")" "$(count_rows "$REC_LONG")" "--recursive matches -r row count"

# ── Claim: --with-metadata adds metadata columns (L275) ──────────────────────
say
say "=== CLAIM: --with-metadata shows all metadata columns vs default (L275) ==="
DEFAULT_TBL="$(dvs status)"
META_TBL="$(dvs status --with-metadata)"
DEFAULT_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$DEFAULT_TBL" | head -1 | tr -d ' ' )"
META_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$META_TBL" | head -1 | tr -d ' ' )"
echo "default header : $DEFAULT_HDR"
echo "metadata header: $META_HDR"
check "|path|status|size|" "$DEFAULT_HDR" "default table columns are path/status/size"
check "|path|status|size|hash|created_by|add_time|compression|message|" "$META_HDR" "--with-metadata adds hash/created_by/add_time/compression/message"

# ── Claim: --json output shape (L269) ────────────────────────────────────────
say
say "=== CLAIM: --json output shape (L269) ==="
JSON="$(dvs status --json)"
echo "$JSON"
# Valid JSON array of 5 objects each with path/status/metadata.
JLEN="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d))' )"
check "5" "$JLEN" "--json emits an array of 5 objects"
JKEYS="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(",".join(sorted(d[0].keys())))' )"
check "metadata,path,status" "$JKEYS" "--json object has path/status/metadata keys"
# json honors filters
JABS="$({ set +x; } 2>/dev/null; dvs status --json --absent | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d), d[0]["status"])' )"
check "1 absent" "$JABS" "--json honors --absent filter"

# ── Claim: exit 1 if a file cannot be inspected (L282) ───────────────────────
say
say "=== CLAIM: exit 1 if a file cannot be inspected (L282) ==="
SIDE=".dvs/data/raw/file_2.bin.dvs"
cp "$SIDE" "$SCRIPT_DIR/sidebak_$RUN_SUFFIX"

# (a) malformed JSON sidecar -> deterministic, root-safe.
printf 'not json{{{' > "$SIDE"
rc=0
ERR_OUT="$(dvs status 2>&1)" || rc=$?
echo "$ERR_OUT"
echo "EXIT(malformed)=$rc"
check "1" "$rc" "malformed .dvs sidecar -> exit 1"
PERFILE="$({ set +x; } 2>/dev/null; grep -c 'Error getting status for data/raw/file_2.bin' <<<"$ERR_OUT" )"
check "1" "$PERFILE" "per-file error reported for the broken sidecar"
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
