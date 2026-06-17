#!/usr/bin/env bash

# Validate CLI-observable `dvs add` claims from specs.md (Script 2 in ui/TODO.md).
# Each claim prints expected vs actual and a PASS/FAIL verdict via check().

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

set -xo pipefail

# ── Setup: one CLI repo + sibling storage ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"
DVS_STORAGE_ABS="$(cd "$DVS_STORAGE" && pwd)"

# A symlink target OUTSIDE the project root lives here (not under DVS_REPO).
OUTSIDE_DIR="$(mktemp -d /tmp/dvs_outside_XXX)"
mkrandfile "$OUTSIDE_DIR/ext.bin" 256

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE_ABS" >/dev/null

# =====================================================================
say
say "=== CLAIM: add --help matches spec block L153-167 (option order) ==="
# Expected reference block from specs.md L153-167.
EXPECTED_HELP='Adds the given files to dvs. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided

Usage: dvs add [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...

Options:
      --json               Output results as JSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
      --glob <GLOB>
  -m, --message <MESSAGE>  An optional message to add
      --dry-run            Show what would be added without making any actual changes
  -h, --help               Print help'
ACTUAL_HELP="$(dvs add --help)"
# Compare with trailing whitespace stripped per line: the spec markdown block
# and clap both emit trailing padding that round-trips inconsistently through
# markdown, so the load-bearing comparison is the visible text + order.
norm() { printf '%s\n' "$1" | sed 's/[[:space:]]*$//'; }
check "$(norm "$EXPECTED_HELP")" "$(norm "$ACTUAL_HELP")" "add --help text matches spec block (trailing-space normalized)"
# Option order specifically: long flags within the Options: block only.
EXP_ORDER="json threads glob message dry-run help"
GOT_ORDER="$(printf '%s\n' "$ACTUAL_HELP" \
  | sed -n '/^Options:/,$p' \
  | sed -n 's/^[[:space:]]*\(-[a-z], \)\{0,1\}--\([a-z-]*\).*/\2/p' \
  | tr '\n' ' ' | sed 's/ $//')"
check "$EXP_ORDER" "$GOT_ORDER" "add --help option ORDER is json,threads,glob,message,dry-run,help"

# =====================================================================
say
say "=== CLAIM: bare directory arg does NOT add (needs a glob) (L130) ==="
mkfiles 1 1K data
rc=0; OUT="$(dvs add data/ 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "bare 'dvs add data/' exits non-zero (1)"
check "YES" "$([ -e .dvs/data/file_1.bin.dvs ] && echo NO || echo YES)" "bare dir add created NO sidecar for data/file_1.bin"

# =====================================================================
say
say "=== CLAIM: -m/--message recorded in .dvs metadata 'message' (L130,L165) ==="
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
say "=== CLAIM: add has NO --recursive option (L131) ==="
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
say "=== CLAIM: best-effort, valid+failing(unreadable) -> valid added, failing reported, exit 1 (L133,L173) ==="
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
say
say "=== CLAIM: missing path -> exit 1 (L173) ==="
rc=0; OUT="$(dvs add definitely_missing.bin 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "exit code 1 when a path is missing"

# NOTE on best-effort vs missing path (spec L133): a *missing* path aborts the
# whole add during resolution (no valid files processed), whereas a *processing*
# failure (unreadable) is best-effort. Documented in the return table.
say
say "--- probe: does a MISSING path abort sibling valid files? (informational) ---"
mkrandfile sibling.bin 256
rc=0; dvs add sibling.bin gone.bin >/dev/null 2>&1 || rc=$?
ABORTED="$([ -e .dvs/sibling.bin.dvs ] && echo NO || echo YES)"
say "missing-path aborts valid sibling? $ABORTED (exit=$rc)"

# =====================================================================
say
say "=== CLAIM: sidecar created at <metadata>/<path>.dvs mirroring path (L136,L311) ==="
mkfiles 1 1K nested/deep
dvs add nested/deep/file_1.bin >/dev/null
check "YES" "$([ -e .dvs/nested/deep/file_1.bin.dvs ] && echo YES || echo NO)" "sidecar mirrors path at .dvs/nested/deep/file_1.bin.dvs"

# =====================================================================
say
say "=== CLAIM: data file gitignored, .dvs sidecar NOT gitignored (L137) ==="
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
say "=== CLAIM: outcome 'copied' for new/changed file (L141) ==="
mkrandfile fresh.bin 256
OC="$(dvs add --json fresh.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$OC" "new file reports outcome=copied (json)"

# =====================================================================
say
say "=== CLAIM: outcome 'present' on re-add; metadata not rewritten, message unchanged (L142-143) ==="
mkrandfile reAdd.bin 256
dvs add -m "first message" reAdd.bin >/dev/null
ADDTIME1="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs"))["add_time"])')"
OC2="$(dvs add --json -m "SECOND message" reAdd.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "present" "$OC2" "unchanged re-add reports outcome=present"
MSG_AFTER="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs")).get("message","<absent>"))')"
check "first message" "$MSG_AFTER" "present re-add did NOT update message (still 'first message')"
ADDTIME2="$(python3 -c 'import json;print(json.load(open(".dvs/reAdd.bin.dvs"))["add_time"])')"
check "$ADDTIME1" "$ADDTIME2" "present re-add did NOT rewrite metadata (add_time unchanged)"

# =====================================================================
say
say "=== CLAIM: symlink resolved before adding (in-project symlink) (L145) ==="
mkrandfile realtarget.bin 256
ln -s realtarget.bin slink.bin
rc=0; OUT="$(dvs add slink.bin 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "adding in-project symlink succeeds (exit 0)"
# resolved => sidecar created for the resolved real target, not the link name
RESOLVED="$([ -e .dvs/realtarget.bin.dvs ] && echo yes || echo no)"
check "yes" "$RESOLVED" "symlink resolved to real target (sidecar at realtarget.bin.dvs)"

# =====================================================================
say
say "=== CLAIM: symlink whose target is OUTSIDE project root is rejected (L145) ==="
ln -s "$OUTSIDE_DIR/ext.bin" outside.bin
rc=0; OUT="$(dvs add outside.bin 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "outside-root symlink add exits 1"
OUTREJ="$(printf '%s' "$OUT" | grep -qiE "outside project|outside the project" && echo yes || echo no)"
check "yes" "$OUTREJ" "outside-root symlink rejected with 'outside project' error"
check "NO" "$([ -e .dvs/outside.bin.dvs ] && echo YES || echo NO)" "no sidecar created for outside-root symlink"

# =====================================================================
say
say "=== CLAIM: atomicity, no .tmp left in storage, no partial metadata (L147-148) ==="
TMPCOUNT="$(find "$DVS_STORAGE_ABS" -name '*.tmp' | wc -l | tr -d ' ')"
check "0" "$TMPCOUNT" "no .tmp leftovers in storage after adds"

# =====================================================================
say
say "=== CLAIM: dvs add *.bin (shell-expanded) works (L170) ==="
mkdir -p shellglob && cd shellglob
mkrandfile s1.bin 100; mkrandfile s2.bin 100
dvs add *.bin >/dev/null
SG="$([ -e ../.dvs/shellglob/s1.bin.dvs ] && [ -e ../.dvs/shellglob/s2.bin.dvs ] && echo yes || echo no)"
check "yes" "$SG" "shell-expanded 'dvs add *.bin' added both files"
cd "$DVS_REPO"

# =====================================================================
say
say "=== CLAIM: --glob '*.bin' (lib-expanded, quoted) works (L171) ==="
mkdir -p libglob && cd libglob
mkrandfile l1.bin 100; mkrandfile l2.bin 100
dvs add --glob '*.bin' >/dev/null
LG="$([ -e ../.dvs/libglob/l1.bin.dvs ] && [ -e ../.dvs/libglob/l2.bin.dvs ] && echo yes || echo no)"
check "yes" "$LG" "lib-expanded --glob '*.bin' added both files"
cd "$DVS_REPO"

# =====================================================================
say
say "=== CLAIM: --dry-run reports outcomes but makes NO changes (L166) ==="
mkrandfile dry.bin 256
OUT="$(dvs add --dry-run dry.bin 2>&1)"
say "$OUT"
check "NO" "$([ -e .dvs/dry.bin.dvs ] && echo YES || echo NO)" "--dry-run created NO .dvs sidecar"
# Confirm no storage blob was created: the blob count must not grow.
BLOBS_BEFORE="$(find "$DVS_STORAGE_ABS" -type f ! -name 'audit.log.jsonl' | wc -l | tr -d ' ')"
dvs add --dry-run dry.bin >/dev/null 2>&1
BLOBS_AFTER="$(find "$DVS_STORAGE_ABS" -type f ! -name 'audit.log.jsonl' | wc -l | tr -d ' ')"
check "$BLOBS_BEFORE" "$BLOBS_AFTER" "--dry-run created NO storage blob (blob count unchanged)"

# =====================================================================
say
say "=== CLAIM: --json output for add (L162) ==="
mkrandfile j.bin 256
JOUT="$(dvs add --json j.bin)"
say "$JOUT"
VALIDJSON="$(printf '%s' "$JOUT" | python3 -c 'import sys,json;d=json.load(sys.stdin);print("yes" if isinstance(d,list) and "outcome" in d[0] and "path" in d[0] else "no")')"
check "yes" "$VALIDJSON" "--json emits a JSON array with path/outcome fields"

# =====================================================================
say
say "=== CLAIM: results sorted alphabetically by path (L178) ==="
mkrandfile zsort.bin 50; mkrandfile asort.bin 50; mkrandfile msort.bin 50
SORTED="$(dvs add --json zsort.bin msort.bin asort.bin | python3 -c 'import sys,json;ps=[r["path"] for r in json.load(sys.stdin)];print("yes" if ps==sorted(ps) else "no:"+",".join(ps))')"
check "yes" "$SORTED" "add results sorted alphabetically by path (gave z,m,a -> a,m,z)"

# =====================================================================
say
say "=== CLAIM: audit entries action=add with file(path+hashes)+compression (L361-372) ==="
mkrandfile audited.bin 256
EXPHASH="$(dvs add --json audited.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["hash"])')"
AUDIT="$DVS_STORAGE_ABS/audit.log.jsonl"
check "YES" "$([ -e "$AUDIT" ] && echo YES || echo NO)" "audit.log.jsonl exists in storage"
AUDIT_OK="$(python3 - "$AUDIT" "$EXPHASH" <<'PY'
import sys, json
path, exphash = sys.argv[1], sys.argv[2]
found = False
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
            found = True
            break
print("yes" if found else "no")
PY
)"
check "yes" "$AUDIT_OK" "audit add entry has file{path,hashes.blake3} + compression for audited.bin"

# =====================================================================
say
echo "=== SUMMARY: $PASS pass, $FAIL fail ==="

# cleanup the outside-root scratch dir (cleanup.sh handles dvs_repo_*/dvs_storage_*)
rm -rf "$OUTSIDE_DIR"
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
