#!/usr/bin/env bash

# Validate CLI-observable `dvs status` claims from specs.md (Script 4 in ui/TODO.md).
# CLI only, verdict-based. Each claim prints expected vs actual and a PASS/FAIL,
# ending with `=== SUMMARY: N pass, M fail ===`.
# Line refs (Lnnn) are to specs.md on main at 7ff0aa0.

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

# Build the five states (L50-54):
#   current : data/file_1.bin, data/raw/file_2.bin, models/v1/file_1.bin
#   modified: data/raw/file_1.bin  (append bytes after add: differs from the
#             metadata and from the baseline add recorded, L485)
#   absent  : data/derived/file_1.bin (delete local copy)
#   stale   : other/stale.bin  (metadata advanced, as after a git pull, while
#             the local file still matches its baseline)
#   unsynced: other/unsynced.bin (metadata arrives for a path dvs never wrote
#             or added in this clone, so there is no baseline, L772-774)
printf 'EXTRA' >> data/raw/file_1.bin
rm data/derived/file_1.bin
cp .dvs/other/donor.bin.dvs .dvs/other/stale.bin.dvs
cp .dvs/other/donor.bin.dvs .dvs/other/unsynced.bin.dvs
rm .dvs/other/donor.bin.dvs other/donor.bin

# ── Claim: status --help matches spec block (L609-632) ───────────────────────
say
say "=== CLAIM: status --help matches spec (L609-632) ==="
HELP="$(dvs status --help)"
EXPECTED_HELP="$(cat <<'EOF'
Gets the status of each file in the current repository

Usage: dvs status [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Paths (files or directories) to check status for

Options:
  -r, --recursive      Recursively include files in subdirectories for given directories. Without this flag, directories return only their direct children
  -g, --glob <GLOB>    Only include the tracked files matching this glob, e.g. '*.csv'
      --current        Include the files that are current
      --absent         Include the files that are absent
      --stale          Include files unchanged since the last successful get, when metadata has advanced
      --modified       Include files changed since the last successful get
      --unsynced       Include differing files whose local baseline is unavailable
      --with-metadata  Show all metadata columns in the table output
  -h, --help           Print help

Global Options:
      --json               Output results as JSON
      --emit-events        Stream operation results and transfer progress as NDJSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
EOF
)"
norm() { printf '%s\n' "$1" | sed 's/[[:space:]]*$//'; }
check "$(norm "$EXPECTED_HELP")" "$(norm "$HELP")" "help text matches spec block (trailing-space normalized)"
# Option flags in the order they appear in --help.
OPT_ORDER="$({ set +x; } 2>/dev/null; grep -oE '\-\-(json|emit-events|threads|recursive|glob|current|absent|stale|modified|unsynced|with-metadata)' <<<"$HELP" | tr '\n' ',' )"
check "--recursive,--glob,--current,--absent,--stale,--modified,--unsynced,--with-metadata,--json,--emit-events,--threads," "$OPT_ORDER" "help option order matches spec"
USAGE="$({ set +x; } 2>/dev/null; grep -oE 'Usage: dvs status \[OPTIONS\] \[PATHS\]\.\.\.' <<<"$HELP" )"
check "Usage: dvs status [OPTIONS] [PATHS]..." "$USAGE" "help usage line matches spec"
SHORTR="$({ set +x; } 2>/dev/null; grep -c '\-r, --recursive' <<<"$HELP" )"
check "1" "$SHORTR" "help documents short -r for --recursive"

# ── Claim: default (no flags) shows ALL tracked files (L634) ─────────────────
say
say "=== CLAIM: default no-flags shows all 7 tracked files regardless of state (L634) ==="
ALL="$(dvs status)"
echo "$ALL"
check "7" "$(count_rows "$ALL")" "default shows all 7 tracked rows"
# Human output ends with a summary line (L637).
check "Checked 7" "$(printf '%s\n' "$ALL" | tail -1)" "human output ends with the summary line 'Checked 7' (L637)"

# ── Claim: five states reported correctly (L48-54) ───────────────────────────
say
say "=== CLAIM: states current/absent/stale/modified/unsynced reported correctly (L48-54) ==="
check "yes" "$(has_row "$ALL" "data/file_1.bin" "current")"           "data/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_2.bin" "current")"       "data/raw/file_2.bin is current"
check "yes" "$(has_row "$ALL" "models/v1/file_1.bin" "current")"      "models/v1/file_1.bin is current"
check "yes" "$(has_row "$ALL" "data/raw/file_1.bin" "modified")"      "data/raw/file_1.bin (edited after add) is modified (L53)"
check "yes" "$(has_row "$ALL" "data/derived/file_1.bin" "absent")"    "data/derived/file_1.bin is absent"
check "yes" "$(has_row "$ALL" "other/stale.bin" "stale")"             "other/stale.bin (metadata advanced, local = baseline) is stale (L52)"
check "yes" "$(has_row "$ALL" "other/unsynced.bin" "unsynced")"       "other/unsynced.bin (differs, no baseline) is unsynced (L54)"

# ── Claim: --current filters to current only (L620, L634-635) ────────────────
say
say "=== CLAIM: --current filters to current only (L620) ==="
CUR="$(dvs status --current)"
echo "$CUR"
check "3" "$(count_rows "$CUR")" "--current shows 3 rows"
check "no" "$(has_row "$CUR" "data/raw/file_1.bin" "modified")" "--current excludes modified"
check "no" "$(has_row "$CUR" "data/derived/file_1.bin" "absent")" "--current excludes absent"

# ── Claim: --absent filters to absent only (L621) ────────────────────────────
say
say "=== CLAIM: --absent filters to absent only (L621) ==="
ABS="$(dvs status --absent)"
echo "$ABS"
check "1" "$(count_rows "$ABS")" "--absent shows 1 row"
check "yes" "$(has_row "$ABS" "data/derived/file_1.bin" "absent")" "--absent shows the absent file"

# ── Claim: --stale, --modified, --unsynced filter to their state (L622-624) ──
say
say "=== CLAIM: --stale filters to stale only (L622) ==="
STA="$(dvs status --stale)"
echo "$STA"
check "1" "$(count_rows "$STA")" "--stale shows 1 row"
check "yes" "$(has_row "$STA" "other/stale.bin" "stale")" "--stale shows the stale file"

say
say "=== CLAIM: --modified filters to modified only (L623) ==="
MOD="$(dvs status --modified)"
echo "$MOD"
check "1" "$(count_rows "$MOD")" "--modified shows 1 row"
check "yes" "$(has_row "$MOD" "data/raw/file_1.bin" "modified")" "--modified shows the modified file"

say
say "=== CLAIM: --unsynced filters to unsynced only (L624) ==="
UNS="$(dvs status --unsynced)"
echo "$UNS"
check "1" "$(count_rows "$UNS")" "--unsynced shows 1 row"
check "yes" "$(has_row "$UNS" "other/unsynced.bin" "unsynced")" "--unsynced shows the unsynced file"

# ── Claim: multiple filters combine as a union (L634-635) ────────────────────
say
say "=== CLAIM: --current --absent combine (union) (L634-635) ==="
UNION="$(dvs status --current --absent)"
echo "$UNION"
check "4" "$(count_rows "$UNION")" "--current --absent shows 3 current + 1 absent = 4 rows"
check "no" "$(has_row "$UNION" "data/raw/file_1.bin" "modified")" "union excludes modified"

# ── Claim: path filter to a single file (L615) ───────────────────────────────
say
say "=== CLAIM: path filter to a single file (L615) ==="
ONE="$(dvs status data/raw/file_2.bin)"
echo "$ONE"
check "1" "$(count_rows "$ONE")" "single-file path filter shows exactly 1 row"
check "yes" "$(has_row "$ONE" "data/raw/file_2.bin" "current")" "single-file path shows that file"

# ── Claim: a path matching no tracked file is dropped silently (L640-642) ────
say
say "=== CLAIM: a path matching no tracked file is dropped silently, exit 0 (L640-642) ==="
rc=0; BOGUS="$(dvs status data/raw/file_2.bin bogus.bin)" || rc=$?
echo "$BOGUS"
check "0" "$rc" "'status good bogus' exits 0"
check "1" "$(count_rows "$BOGUS")" "'status good bogus' shows only the matching path"

# ── Claim: directory path, non-recursive = direct children only (L618) ───────
say
say "=== CLAIM: directory non-recursive = direct children only (L618) ==="
DIR="$(dvs status data/)"
echo "$DIR"
check "1" "$(count_rows "$DIR")" "data/ non-recursive shows only direct child (1 row)"
check "yes" "$(has_row "$DIR" "data/file_1.bin" "current")" "data/ non-recursive includes direct child data/file_1.bin"
check "no" "$(has_row "$DIR" "data/raw/file_2.bin" "current")" "data/ non-recursive excludes data/raw descendant"

# ── Claim: -r/--recursive includes all descendants (L618) ────────────────────
say
say "=== CLAIM: -r/--recursive includes all descendants of a directory (L618) ==="
REC="$(dvs status -r data/)"
echo "$REC"
check "4" "$(count_rows "$REC")" "-r data/ shows all 4 descendants (top + raw1 + raw2 + derived)"
check "yes" "$(has_row "$REC" "data/raw/file_2.bin" "current")" "-r data/ includes nested data/raw/file_2.bin"
check "yes" "$(has_row "$REC" "data/derived/file_1.bin" "absent")" "-r data/ includes nested data/derived/file_1.bin"
# long form parity
REC_LONG="$(dvs status --recursive data/)"
check "$(count_rows "$REC")" "$(count_rows "$REC_LONG")" "--recursive matches -r row count"

# ── Claim: --with-metadata adds metadata columns (L625) ──────────────────────
say
say "=== CLAIM: --with-metadata shows all metadata columns vs default (L625) ==="
DEFAULT_TBL="$(dvs status)"
META_TBL="$(dvs status --with-metadata)"
DEFAULT_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$DEFAULT_TBL" | head -1 | tr -d ' ' )"
META_HDR="$({ set +x; } 2>/dev/null; grep '^| path' <<<"$META_TBL" | head -1 | tr -d ' ' )"
echo "default header : $DEFAULT_HDR"
echo "metadata header: $META_HDR"
check "|path|status|size|" "$DEFAULT_HDR" "default table columns are path/status/size"
check "|path|status|size|hash|created_by|add_time|compression|message|" "$META_HDR" "--with-metadata adds hash/created_by/add_time/compression/message"

# ── Claim: --json output shape (L89-99, L123-127) ────────────────────────────
say
say "=== CLAIM: --json output shape (L89-99, L123-127) ==="
JSON="$(dvs status --json)"
echo "$JSON"
JENV="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'print(d["v"], d["command"], d["outcome"], d["error"], d["counts"]["total"], d["counts"]["succeeded"], d["counts"]["failed"])' )"
check "1 status complete None 7 7 0" "$JENV" "--json is a v1 envelope: command=status, outcome=complete, counts total=succeeded=7"
JLEN="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'print(len(d["files"]))' )"
check "7" "$JLEN" "--json files has 7 rows"
# Rows have fixed keys (L114-116, L124-127); extra keys are tolerated (L86).
JKEYS="$({ set +x; } 2>/dev/null; printf '%s' "$JSON" | jq_py 'need={"path","status","metadata","working","baseline","code","message","hint"};print(",".join(sorted(need-set(d["files"][0]))) or "ok")' )"
check "ok" "$JKEYS" "--json row has path/status/metadata/working/baseline/code/message/hint keys"
# Row facts behind the states (L124-127): absent -> working.state absent;
# stale -> local matches the baseline, not the metadata; unsynced -> no baseline.
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

# ── Claim: exit 1 if a file cannot be inspected (L638, L646-648) ─────────────
say
say "=== CLAIM: exit 1 if a file cannot be inspected (L638) ==="
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
# The same failure as a --json row: status=error and code bad_metadata (L124, L179-180).
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
