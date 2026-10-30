#!/usr/bin/env bash

# Validate CLI-observable `dvs add` claims from specs.md (Script 2 in ui/TODO.md).
# Each claim prints expected vs actual and a PASS/FAIL verdict via check().
# Line refs (Lnnn) are to specs.md on main at 7ff0aa0.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

PASS=0
FAIL=0
check() {
  if [ "$1" = "$2" ]; then
    echo "PASS: $3"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $3 | want=[$1] got=[$2]"
    FAIL=$((FAIL + 1))
  fi
}

# Run a python expression over the --json envelope on stdin. Prints
# <parse-error> instead of aborting, so the script always reaches SUMMARY.
jq_py() { python3 -c "import sys,json;d=json.load(sys.stdin);$1" 2>/dev/null || echo "<parse-error>"; }

set -xo pipefail

# ── Setup: one CLI repo + sibling storage ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"
DVS_STORAGE_ABS="$(cd "$DVS_STORAGE" && pwd)"

# A symlink target OUTSIDE the project root lives here (not under DVS_REPO).
OUTSIDE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dvs_outside_XXX")"
mkrandfile "$OUTSIDE_DIR/ext.bin" 256

cd "$DVS_REPO"
git init -q
dvs init local "$DVS_STORAGE_ABS" >/dev/null

# =====================================================================
say
say "=== CLAIM: add --help matches spec block L405-423 (option order) ==="
# Expected reference block from specs.md L405-423.
EXPECTED_HELP='Adds the given files to dvs. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided

Usage: dvs add [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Files to add, or directories to search with --glob

Options:
      --glob <GLOB>        Add the files matching this glob, e.g. '"'"'*.csv'"'"'
  -m, --message <MESSAGE>  An optional message to add
      --dry-run            Show what would be added without making any actual changes
  -h, --help               Print help

Global Options:
      --json               Output results as JSON
      --emit-events        Stream operation results and transfer progress as NDJSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)'
ACTUAL_HELP="$(dvs add --help)"
# Compare with trailing whitespace stripped per line: the spec markdown block
# and clap both emit trailing padding that round-trips inconsistently through
# markdown, so the load-bearing comparison is the visible text + order.
norm() { printf '%s\n' "$1" | sed 's/[[:space:]]*$//'; }
check "$(norm "$EXPECTED_HELP")" "$(norm "$ACTUAL_HELP")" "add --help text matches spec block (trailing-space normalized)"
# Option order specifically: long flags from the Options: block to the end
# (local options, then the Global Options block).
EXP_ORDER="glob message dry-run help json emit-events threads"
GOT_ORDER="$(printf '%s\n' "$ACTUAL_HELP" \
  | sed -n '/^Options:/,$p' \
  | sed -n 's/^[[:space:]]*\(-[a-z], \)\{0,1\}--\([a-z-]*\).*/\2/p' \
  | tr '\n' ' ' | sed 's/ $//')"
check "$EXP_ORDER" "$GOT_ORDER" "add --help option ORDER is glob,message,dry-run,help then globals json,emit-events,threads"

# =====================================================================
# A directory without --glob is an invalid input path: the batch is refused
# with invalid_arguments and exit 2 (#240 fail-fast, then the exit-code table).
say
say "=== CLAIM: bare directory arg does NOT add (needs a glob), invalid_arguments, exit 2 (L381, L384, L428) ==="
mkfiles 1 1K data
rc=0; OUT="$(dvs add data/ 2>&1)" || rc=$?
say "$OUT"
check "2" "$rc" "bare 'dvs add data/' exits 2"
check "YES" "$([ -e .dvs/data/file_1.bin.dvs ] && echo NO || echo YES)" "bare dir add created NO sidecar for data/file_1.bin"
JOUT="$(dvs add --json data/ 2>/dev/null)" || true
JCODE="$(printf '%s' "$JOUT" | jq_py 'print((d.get("error") or {}).get("code"))')"
check "invalid_arguments" "$JCODE" "bare dir add --json error.code is invalid_arguments"

# =====================================================================
say
say "=== CLAIM: -m/--message recorded in .dvs metadata 'message' (L381, L415, L695) ==="
mkrandfile withmsg.bin 512
dvs add -m "hello from spec test" withmsg.bin >/dev/null
GOTMSG="$(python3 -c 'import json;print(json.load(open(".dvs/withmsg.bin.dvs")).get("message","<absent>"))')"
check "hello from spec test" "$GOTMSG" "-m message recorded in sidecar message field"
# And: message omitted when not provided.
mkrandfile nomsg.bin 512
dvs add nomsg.bin >/dev/null
HASMSG="$(python3 -c 'import json;print("yes" if "message" in json.load(open(".dvs/nomsg.bin.dvs")) else "no")')"
check "no" "$HASMSG" "message field omitted from sidecar when no -m given"

# =====================================================================
say
say "=== CLAIM: add has NO --recursive option (L382, L806) ==="
rc=0; OUT="$(dvs add --recursive withmsg.bin 2>&1)" || rc=$?
say "$OUT"
ISUNKNOWN="$(printf '%s' "$OUT" | grep -qiE "unexpected argument '--recursive'|unrecognized" && echo yes || echo no)"
check "yes" "$ISUNKNOWN" "--recursive rejected as unknown argument"
check "0" "$([ "$rc" -ne 0 ] && echo 0 || echo 1)" "--recursive exits non-zero"
rc=0; OUT="$(dvs add -r withmsg.bin 2>&1)" || rc=$?
ISUNKNOWN_R="$(printf '%s' "$OUT" | grep -qiE "unexpected argument '-r'|unexpected argument '--recursive'|tip:" && echo yes || echo no)"
check "yes" "$ISUNKNOWN_R" "-r short flag also rejected as unknown argument"

# =====================================================================
say
say "=== CLAIM: best-effort, valid+failing(unreadable) -> valid added, failing reported, exit 1 (L386, L429) ==="
mkrandfile good.bin 256
mkrandfile noperm.bin 256
chmod 000 noperm.bin
rc=0; OUT="$(dvs add good.bin noperm.bin 2>&1)" || rc=$?
say "$OUT"
chmod 644 noperm.bin
check "YES" "$([ -e .dvs/good.bin.dvs ] && echo YES || echo NO)" "best-effort: valid good.bin still added alongside an unreadable file"
REPORTED="$(printf '%s' "$OUT" | grep -qiE "noperm.bin" && echo yes || echo no)"
check "yes" "$REPORTED" "best-effort: failing noperm.bin reported in output"
check "1" "$rc" "exit code 1 when a file (unreadable) could not be added"

# =====================================================================
# A path that does not exist is an invalid input path (L384), and an invalid
# input path is invalid_arguments with exit 2 (L428).
say
say "=== CLAIM: missing path is an invalid input path, exit 2 (L384, L428) ==="
rc=0; OUT="$(dvs add definitely_missing.bin 2>&1)" || rc=$?
say "$OUT"
check "2" "$rc" "exit code 2 when a path is missing"

# =====================================================================
# An unresolvable input path refuses the WHOLE batch (#240): no valid sibling
# is added, the missing path is named, and the error is invalid_arguments.
say
say "=== CLAIM: all-or-nothing, any invalid input path refuses the whole batch (L384, L428, #240) ==="
mkrandfile sibling.bin 256
rc=0; JOUT="$(dvs add --json sibling.bin gone.bin 2>/dev/null)" || rc=$?
say "$JOUT"
check "NO" "$([ -e .dvs/sibling.bin.dvs ] && echo YES || echo NO)" "all-or-nothing: valid sibling.bin NOT added when a sibling path is missing"
REPORTED_MISS="$(printf '%s' "$JOUT" | jq_py 'print("yes" if "gone.bin" in ((d.get("error") or {}).get("message") or "") else "no")')"
check "yes" "$REPORTED_MISS" "the missing gone.bin is named in the refusal message"
JCODE="$(printf '%s' "$JOUT" | jq_py 'print((d.get("error") or {}).get("code"))')"
check "invalid_arguments" "$JCODE" "refusal for a missing path has error.code invalid_arguments"
# All input is evaluated and every invalid path reported (L73, L384).
mkdir -p emptydir
rc=0; OUT="$(dvs add sibling.bin gone.bin emptydir 2>&1)" || rc=$?
say "$OUT"
BOTH="$(printf '%s' "$OUT" | grep -q 'gone.bin' && printf '%s' "$OUT" | grep -q 'emptydir' && echo yes || echo no)"
check "yes" "$BOTH" "refusal names every invalid path (missing gone.bin AND directory emptydir)"

# =====================================================================
say
say "=== CLAIM: sidecar created at <metadata>/<path>.dvs mirroring path (L388, L672-673) ==="
mkfiles 1 1K nested/deep
dvs add nested/deep/file_1.bin >/dev/null
check "YES" "$([ -e .dvs/nested/deep/file_1.bin.dvs ] && echo YES || echo NO)" "sidecar mirrors path at .dvs/nested/deep/file_1.bin.dvs"

# =====================================================================
say
say "=== CLAIM: data file gitignored, .dvs sidecar NOT gitignored (L389, L791-792) ==="
mkrandfile gi.bin 256
dvs add gi.bin >/dev/null
GI_DATA="$(grep -qxF '/gi.bin' .gitignore && echo yes || echo no)"
check "yes" "$GI_DATA" "data file gi.bin has /gi.bin entry in .gitignore"
GI_SIDECAR="$(grep -qE 'gi.bin.dvs' .gitignore && echo yes || echo no)"
check "no" "$GI_SIDECAR" ".dvs sidecar NOT present in .gitignore"
# sidecar would be git-trackable (not ignored)
SIDECAR_IGNORED="$(git check-ignore .dvs/gi.bin.dvs >/dev/null 2>&1 && echo yes || echo no)"
check "no" "$SIDECAR_IGNORED" "git does not ignore the .dvs sidecar"

# =====================================================================
say
say "=== CLAIM: outcome 'copied' for new/changed file (L393, L123) ==="
mkrandfile fresh.bin 256
OC="$(dvs add --json fresh.bin | jq_py 'print(d["files"][0]["outcome"])')"
check "copied" "$OC" "new file reports outcome=copied (json)"

# =====================================================================
say
say "=== CLAIM: outcome 'present' on re-add; metadata not rewritten, message unchanged (L394-395) ==="
mkrandfile reAdd.bin 256
dvs add -m "first message" reAdd.bin >/dev/null
ADDTIME1="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs"))["add_time"])')"
OC2="$(dvs add --json -m "SECOND message" reAdd.bin | jq_py 'print(d["files"][0]["outcome"])')"
check "present" "$OC2" "unchanged re-add reports outcome=present"
MSG_AFTER="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs")).get("message","<absent>"))')"
check "first message" "$MSG_AFTER" "present re-add did NOT update message (still 'first message')"
ADDTIME2="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs"))["add_time"])')"
check "$ADDTIME1" "$ADDTIME2" "present re-add did NOT rewrite metadata (add_time unchanged)"

# =====================================================================
say
say "=== CLAIM: symlink resolved before adding (in-project symlink) (L397) ==="
mkrandfile realtarget.bin 256
ln -s realtarget.bin slink.bin
rc=0; OUT="$(dvs add slink.bin 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "adding in-project symlink succeeds (exit 0)"
# resolved => sidecar created for the resolved real target, not the link name
RESOLVED="$([ -e .dvs/realtarget.bin.dvs ] && echo yes || echo no)"
check "yes" "$RESOLVED" "symlink resolved to real target (sidecar at realtarget.bin.dvs)"

# =====================================================================
# An explicit path resolving outside the project is an invalid input path:
# the batch is refused with exit 2 (L397, L428).
say
say "=== CLAIM: symlink whose target is OUTSIDE project root is rejected, exit 2 (L397, L428) ==="
ln -s "$OUTSIDE_DIR/ext.bin" outside.bin
rc=0; OUT="$(dvs add outside.bin 2>&1)" || rc=$?
say "$OUT"
check "2" "$rc" "outside-root symlink add exits 2"
OUTREJ="$(printf '%s' "$OUT" | grep -qiE "outside project|outside the project" && echo yes || echo no)"
check "yes" "$OUTREJ" "outside-root symlink rejected with 'outside project' error"
check "NO" "$([ -e .dvs/outside.bin.dvs ] && echo YES || echo NO)" "no sidecar created for outside-root symlink"

# =====================================================================
say
say "=== CLAIM: atomicity, no .tmp left in storage, no partial metadata (L399-400, L707-709) ==="
TMPCOUNT="$(find "$DVS_STORAGE_ABS" -name '*.tmp' | wc -l | tr -d ' ')"
check "0" "$TMPCOUNT" "no .tmp leftovers in storage after adds"

# =====================================================================
say
say "=== CLAIM: dvs add *.bin (shell-expanded) works (L425) ==="
mkdir -p shellglob && cd shellglob
mkrandfile s1.bin 100; mkrandfile s2.bin 100
dvs add *.bin >/dev/null
SG="$([ -e ../.dvs/shellglob/s1.bin.dvs ] && [ -e ../.dvs/shellglob/s2.bin.dvs ] && echo yes || echo no)"
check "yes" "$SG" "shell-expanded 'dvs add *.bin' added both files"
cd "$DVS_REPO"

# =====================================================================
say
say "=== CLAIM: --glob '*.bin' (lib-expanded, quoted) works (L426) ==="
mkdir -p libglob && cd libglob
mkrandfile l1.bin 100; mkrandfile l2.bin 100
dvs add --glob '*.bin' >/dev/null
LG="$([ -e ../.dvs/libglob/l1.bin.dvs ] && [ -e ../.dvs/libglob/l2.bin.dvs ] && echo yes || echo no)"
check "yes" "$LG" "lib-expanded --glob '*.bin' added both files"
cd "$DVS_REPO"

# =====================================================================
say
say "=== CLAIM: --dry-run reports outcomes but makes NO changes (L44, L416) ==="
mkrandfile dry.bin 256
OUT="$(dvs add --dry-run dry.bin 2>&1)"
say "$OUT"
check "NO" "$([ -e .dvs/dry.bin.dvs ] && echo YES || echo NO)" "--dry-run created NO .dvs sidecar"
# Confirm no storage blob was created: the blob count must not grow.
BLOBS_BEFORE="$(find "$DVS_STORAGE_ABS" -type f ! -name 'audit.log.jsonl' | wc -l | tr -d ' ')"
DRY_JSON="$(dvs add --dry-run --json dry.bin 2>/dev/null)"
BLOBS_AFTER="$(find "$DVS_STORAGE_ABS" -type f ! -name 'audit.log.jsonl' | wc -l | tr -d ' ')"
check "$BLOBS_BEFORE" "$BLOBS_AFTER" "--dry-run created NO storage blob (blob count unchanged)"
DRY_FIELDS="$(printf '%s' "$DRY_JSON" | jq_py 'print(d["dry_run"], d["files"][0]["outcome"], d["bytes"]["transferred"])')"
check "True copied 0" "$DRY_FIELDS" "--dry-run --json has dry_run=true, would-be outcome=copied, bytes.transferred=0 (L91, L112)"

# =====================================================================
say
say "=== CLAIM: --json output for add is the v1 envelope (L89-99, L109-123) ==="
mkrandfile j.bin 256
JOUT="$(dvs add --json j.bin)"
say "$JOUT"
VALIDJSON="$(printf '%s' "$JOUT" | python3 -c '
import sys, json
d = json.load(sys.stdin)
bad = []
for k, v in (("v", 1), ("command", "add"), ("outcome", "complete"), ("dry_run", False), ("error", None)):
    if d.get(k, "<missing>") != v:
        bad.append("%s=%r" % (k, d.get(k, "<missing>")))
for k in ("counts", "bytes", "files", "started_at", "finished_at", "duration_ms"):
    if k not in d:
        bad.append("missing " + k)
c = d.get("counts", {})
if set(c) < {"total", "succeeded", "failed", "cancelled"} or c.get("total") != c.get("succeeded", 0) + c.get("failed", 0) + c.get("cancelled", 0):
    bad.append("counts=%r" % c)
b = d.get("bytes", {})
if not {"total", "to_transfer", "already_stored", "transferred"} <= set(b) or b.get("to_transfer", 0) + b.get("already_stored", 0) != b.get("total"):
    bad.append("bytes=%r" % b)
rows = d.get("files") or [{}]
missing = {"path", "outcome", "hash", "size", "stored_size", "duration_ms", "code", "message", "hint"} - set(rows[0])
if missing:
    bad.append("row missing %s" % sorted(missing))
print("yes" if not bad else ";".join(bad))
' 2>&1 || echo "<parse-error>")"
check "yes" "$VALIDJSON" "--json emits a v1 envelope: counts, bytes, files rows with the spec row keys"

# =====================================================================
say
say "=== CLAIM: results sorted alphabetically by path (L434) ==="
mkrandfile zsort.bin 50; mkrandfile asort.bin 50; mkrandfile msort.bin 50
SORTED="$(dvs add --json zsort.bin msort.bin asort.bin | jq_py 'ps=[r["path"] for r in d["files"]];print("yes" if ps==sorted(ps) else "no:"+",".join(ps))')"
check "yes" "$SORTED" "add results sorted alphabetically by path (gave z,m,a -> a,m,z)"

# =====================================================================
say
say "=== CLAIM: audit entries action=add with file(path+hashes)+compression+message (L722-734) ==="
mkrandfile audited.bin 256
EXPHASH="$(dvs add --json -m "audit message" audited.bin | jq_py 'print(d["files"][0]["hash"])')"
AUDIT="$DVS_STORAGE_ABS/audit.log.jsonl"
check "YES" "$([ -e "$AUDIT" ] && echo YES || echo NO)" "audit.log.jsonl exists in storage"
AUDIT_OK="$(python3 - "$AUDIT" "$EXPHASH" <<'PY'
import sys, json
path, exphash = sys.argv[1], sys.argv[2]
found = "no"
for line in open(path):
    line = line.strip()
    if not line:
        continue
    e = json.loads(line)
    act = e.get("action", {})
    if "add" in act:
        a = act["add"]
        f = a.get("file", {})
        if (f.get("hashes", {}).get("blake3") == exphash
                and "path" in f
                and a.get("compression") in ("zstd", "none")):
            found = "yes" if a.get("message") == "audit message" else "no message: %r" % a.get("message")
            break
print(found)
PY
)"
check "yes" "$AUDIT_OK" "audit add entry has file{path,hashes.blake3} + compression + message for audited.bin"

# =====================================================================
say
echo "=== SUMMARY: $PASS pass, $FAIL fail ==="

# cleanup the outside-root scratch dir (cleanup.sh handles dvs_repo_*/dvs_storage_*)
rm -rf "$OUTSIDE_DIR"
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
