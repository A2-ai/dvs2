#!/usr/bin/env bash

# Exhaustive CLI scenario coverage for `dvs add` (cases_add.sh in ui/SCENARIOS.md).
# Each scenario prints expected vs actual and a PASS/FAIL verdict via check(),
# asserting the SPEC-CORRECT behavior. Tracked divergences are labeled with their
# issue number so a FAIL reads as a known bug, not a test defect.

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

# Count storage blobs (everything except the audit log).
blob_count() {
  find "$DVS_STORAGE_ABS" -type f ! -name 'audit.log.jsonl' | wc -l | tr -d ' '
}

set -xo pipefail

# ── Setup: one CLI repo + sibling storage (storage OUTSIDE the repo) ──

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
say "=== add a new file -> outcome copied (L154) ==="
mkrandfile fresh.bin 256
JOUT="$(dvs add --json fresh.bin)"
say "$JOUT"
OC="$(printf '%s' "$JOUT" | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$OC" "new file reports outcome=copied"
check "YES" "$([ -e .dvs/fresh.bin.dvs ] && echo YES || echo NO)" "new file sidecar created at .dvs/fresh.bin.dvs"

# =====================================================================
say
say "=== re-add UNCHANGED file -> present, metadata NOT rewritten (L155-156) ==="
HASH1="$(python3 -c 'import json;print(json.load(open(".dvs/fresh.bin.dvs"))["hashes"]["blake3"])')"
ADDT1="$(python3 -c 'import json;print(json.load(open(".dvs/fresh.bin.dvs"))["add_time"])')"
OC2="$(dvs add --json fresh.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "present" "$OC2" "unchanged re-add reports outcome=present"
ADDT2="$(python3 -c 'import json;print(json.load(open(".dvs/fresh.bin.dvs"))["add_time"])')"
check "$ADDT1" "$ADDT2" "present re-add did NOT rewrite metadata (add_time unchanged)"

# =====================================================================
say
say "=== modify locally then re-add -> copied, new hash + size + add_time (L154) ==="
mkrandfile mod.bin 128
dvs add mod.bin >/dev/null
MH1="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["hashes"]["blake3"])')"
MS1="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["size"])')"
MT1="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["add_time"])')"
sleep 1
mkrandfile mod.bin 256
OCM="$(dvs add --json mod.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$OCM" "modified file re-add reports outcome=copied"
MH2="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["hashes"]["blake3"])')"
MS2="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["size"])')"
MT2="$(python3 -c 'import json;print(json.load(open(".dvs/mod.bin.dvs"))["add_time"])')"
check "yes" "$([ "$MH1" != "$MH2" ] && echo yes || echo no)" "modified file got a NEW hash"
check "yes" "$([ "$MS1" != "$MS2" ] && echo yes || echo no)" "modified file got a NEW size (128 -> 256)"
check "yes" "$([ "$MT1" != "$MT2" ] && echo yes || echo no)" "modified file got a NEW add_time"

# =====================================================================
say
say "=== add a file that does not exist -> exit 1, reported (L186) ==="
rc=0; OUT="$(dvs add definitely_missing.bin 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "missing path exits 1"
MISSREP="$(printf '%s' "$OUT" | grep -qiE 'definitely_missing.bin|not found' && echo yes || echo no)"
check "yes" "$MISSREP" "missing path reported in output"

# =====================================================================
say
say "=== several files, ONE missing -> all-or-nothing: nothing added, missing reported, exit 1 (#219/#240) ==="
# SPEC (L146,L186): when any input path is invalid the whole batch is refused and
# nothing is added. #240 made add fail-fast; the old best-effort behavior is gone.
mkrandfile sibling.bin 256
rc=0; OUT="$(dvs add sibling.bin gone.bin 2>&1)" || rc=$?
say "$OUT"
SIBADDED="$([ -e .dvs/sibling.bin.dvs ] && echo YES || echo NO)"
check "NO" "$SIBADDED" "#219/#240 all-or-nothing: valid sibling.bin NOT added when a sibling path is missing"
MISSREP2="$(printf '%s' "$OUT" | grep -qiE 'gone.bin|not found' && echo yes || echo no)"
check "yes" "$MISSREP2" "#219 missing gone.bin reported in output"
check "1" "$rc" "#219 exit 1 when a path is missing"

# =====================================================================
say
say "=== bare directory (no glob) -> not added (L143) ==="
mkfiles 1 1K data
rc=0; OUT="$(dvs add data/ 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "bare 'dvs add data/' exits non-zero"
check "YES" "$([ -e .dvs/data/file_1.bin.dvs ] && echo NO || echo YES)" "bare dir add created NO sidecar for data/file_1.bin"

# =====================================================================
say
say "=== -m message recorded in sidecar (L143,L178,L343) ==="
mkrandfile withmsg.bin 256
dvs add -m "hello from cases_add" withmsg.bin >/dev/null
GOTMSG="$(python3 -c 'import json;print(json.load(open(".dvs/withmsg.bin.dvs")).get("message","<absent>"))')"
check "hello from cases_add" "$GOTMSG" "-m message recorded in sidecar message field"
# message omitted when not provided (L352)
mkrandfile nomsg.bin 256
dvs add nomsg.bin >/dev/null
HASMSG="$(python3 -c 'import json;print("yes" if "message" in json.load(open(".dvs/nomsg.bin.dvs")) else "no")')"
check "no" "$HASMSG" "message field omitted from sidecar when no -m given"

# =====================================================================
say
say "=== re-add unchanged file with NEW -m -> present, message NOT updated (L155-156) ==="
mkrandfile remsg.bin 256
dvs add -m "first message" remsg.bin >/dev/null
OCR="$(dvs add --json -m "SECOND message" remsg.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "present" "$OCR" "unchanged re-add with new -m reports present"
MSGAFTER="$(python3 -c 'import json;print(json.load(open(".dvs/remsg.bin.dvs")).get("message","<absent>"))')"
check "first message" "$MSGAFTER" "present re-add did NOT update message (still 'first message')"

# =====================================================================
say
say "=== empty 0-byte file -> copied, size 0 (L348) ==="
: > empty.bin
EOUT="$(dvs add --json empty.bin)"
say "$EOUT"
EOC="$(printf '%s' "$EOUT" | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$EOC" "0-byte file reports outcome=copied"
ESZ="$(python3 -c 'import json;print(json.load(open(".dvs/empty.bin.dvs"))["size"])')"
check "0" "$ESZ" "0-byte file sidecar records size=0"

# =====================================================================
say
say "=== filename with SPACES and UNICODE (L) ==="
mkrandfile "my file.bin" 64
mkrandfile "café_数据.bin" 64
dvs add "my file.bin" "café_数据.bin" >/dev/null
SPOK="$([ -e ".dvs/my file.bin.dvs" ] && echo yes || echo no)"
check "yes" "$SPOK" "file with spaces added (sidecar 'my file.bin.dvs')"
UNIOK="$([ -e ".dvs/café_数据.bin.dvs" ] && echo yes || echo no)"
check "yes" "$UNIOK" "file with unicode name added (sidecar 'café_数据.bin.dvs')"

# =====================================================================
say
say "=== read-only 0444 file -> succeeds (L) ==="
mkrandfile ro.bin 128
chmod 0444 ro.bin
rc=0; OUT="$(dvs add --json ro.bin 2>&1)" || rc=$?
say "$OUT"
chmod 644 ro.bin
check "0" "$rc" "read-only 0444 file add exits 0"
check "YES" "$([ -e .dvs/ro.bin.dvs ] && echo YES || echo NO)" "read-only 0444 file sidecar created"

# =====================================================================
say
say "=== unreadable 000 file -> reported, siblings added, exit 1 (best-effort) (L146,L186) ==="
mkrandfile good.bin 128
mkrandfile noperm.bin 128
chmod 000 noperm.bin
rc=0; OUT="$(dvs add good.bin noperm.bin 2>&1)" || rc=$?
say "$OUT"
chmod 644 noperm.bin
check "YES" "$([ -e .dvs/good.bin.dvs ] && echo YES || echo NO)" "best-effort: valid good.bin added alongside an unreadable file"
NPREP="$(printf '%s' "$OUT" | grep -qiE 'noperm.bin|permission denied' && echo yes || echo no)"
check "yes" "$NPREP" "best-effort: unreadable noperm.bin reported in output"
check "1" "$rc" "exit 1 when an unreadable file could not be added"

# =====================================================================
say
say "=== two DIFFERENT-named identical-content files, SEPARATE invocations -> dedup, one blob (L358) ==="
mkrandfile dup_a.bin 200
cp dup_a.bin dup_b.bin
BEF="$(blob_count)"
dvs add dup_a.bin >/dev/null
MID="$(blob_count)"
dvs add dup_b.bin >/dev/null
AFT="$(blob_count)"
check "1" "$((MID - BEF))" "first of identical pair added exactly one new blob"
check "0" "$((AFT - MID))" "second identical-content file (separate invocation) added NO new blob (dedup)"
DA_HASH="$(python3 -c 'import json;print(json.load(open(".dvs/dup_a.bin.dvs"))["hashes"]["blake3"])')"
DB_HASH="$(python3 -c 'import json;print(json.load(open(".dvs/dup_b.bin.dvs"))["hashes"]["blake3"])')"
check "$DA_HASH" "$DB_HASH" "both sidecars reference the same blake3 (content-addressed dedup)"

# =====================================================================
say
say "=== two identical-content files in ONE invocation -> dedup, both copied, exit 0 (#216) ==="
# SPEC (L154,L358): both files are new paths, so both should report copied and
# share one content-addressed blob. We assert the spec-correct outcome; the
# known race (#216) is two concurrent writers renaming the same .tmp blob.
mkrandfile one_a.bin 200
cp one_a.bin one_b.bin
B216="$(blob_count)"
rc=0; OUT="$(dvs add --json one_a.bin one_b.bin 2>&1)" || rc=$?
say "$OUT"
A216="$(blob_count)"
# Parse only JSON lines (stderr may interleave a human "Error:" line on the race).
OUTCOMES="$(printf '%s\n' "$OUT" | python3 -c '
import sys, json
data = None
for line in sys.stdin:
    line = line.strip()
    if line.startswith("["):
        try:
            data = json.loads(line)
        except Exception:
            pass
if data is None:
    print("PARSE_FAIL")
else:
    print(",".join(sorted(r.get("outcome", "ERR:"+r.get("error","?")[:20]) for r in data)))
' 2>/dev/null || echo PARSE_FAIL)"
check "copied,copied" "$OUTCOMES" "#216 both identical files in one invocation report copied"
check "1" "$((A216 - B216))" "#216 identical pair in one invocation added exactly ONE shared blob"
check "0" "$rc" "#216 same-invocation identical add exits 0"
if [ "$OUTCOMES" != "copied,copied" ] || [ "$rc" != "0" ]; then
  say "NOTE(#216): same-invocation identical-content add hit the known race:"
  say "NOTE(#216): concurrent writers rename the same '<hash>.tmp' blob; the loser"
  say "NOTE(#216): errors 'failed to rename ... No such file or directory'. Tracked bug."
fi

# =====================================================================
say
say "=== same path twice in one invocation -> deduped args, single result, exit 0 ==="
mkrandfile twice.bin 100
rc=0; TOUT="$(dvs add --json twice.bin twice.bin 2>&1)" || rc=$?
say "$TOUT"
check "0" "$rc" "same path twice exits 0"
NRES="$(printf '%s' "$TOUT" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
check "1" "$NRES" "same path twice yields a single result row (arg dedup, not a race)"

# =====================================================================
say
say "=== nested-path file -> sidecar mirrors path (L149,L330) ==="
mkfiles 1 1K nested/deep
dvs add nested/deep/file_1.bin >/dev/null
check "YES" "$([ -e .dvs/nested/deep/file_1.bin.dvs ] && echo YES || echo NO)" "sidecar mirrors path at .dvs/nested/deep/file_1.bin.dvs"

# =====================================================================
say
say "=== --dry-run -> outcomes reported, NO sidecar, NO blob, NO gitignore change (L40,L179) ==="
mkrandfile dry.bin 128
DBEF="$(blob_count)"
OUT="$(dvs add --dry-run dry.bin 2>&1)"
say "$OUT"
DAFT="$(blob_count)"
check "NO" "$([ -e .dvs/dry.bin.dvs ] && echo YES || echo NO)" "--dry-run created NO .dvs sidecar"
check "$DBEF" "$DAFT" "--dry-run created NO storage blob (blob count unchanged)"
DGI="$(grep -qxF '/dry.bin' .gitignore 2>/dev/null && echo yes || echo no)"
check "no" "$DGI" "--dry-run made NO .gitignore entry for dry.bin"
# --dry-run still reports the would-be outcome via --json.
DJ="$(dvs add --dry-run --json dry.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$DJ" "--dry-run --json reports outcome=copied for the new file"

# =====================================================================
say
say "=== --json shape: array of {path, outcome, hash, size, ...} (L175) ==="
mkrandfile shape.bin 128
JS="$(dvs add --json shape.bin)"
say "$JS"
SHAPE="$(printf '%s' "$JS" | python3 -c '
import sys, json
d = json.load(sys.stdin)
ok = isinstance(d, list) and all(k in d[0] for k in ("path", "outcome", "hash", "size"))
print("yes" if ok else "no")
')"
check "yes" "$SHAPE" "--json emits array with path/outcome/hash/size fields"

# =====================================================================
say
say "=== results sorted alphabetically by path (L191) ==="
mkrandfile zsort.bin 50; mkrandfile asort.bin 50; mkrandfile msort.bin 50
SORTED="$(dvs add --json zsort.bin msort.bin asort.bin | python3 -c 'import sys,json;ps=[r["path"] for r in json.load(sys.stdin)];print("yes" if ps==sorted(ps) else "no:"+",".join(ps))')"
check "yes" "$SORTED" "add results sorted alphabetically by path (gave z,m,a -> a,m,z)"

# =====================================================================
say
say "=== gitignore gets /<name>, no duplicate on re-add (L443-444) ==="
mkrandfile gi.bin 128
dvs add gi.bin >/dev/null
GICOUNT1="$(grep -cxF '/gi.bin' .gitignore || true)"
check "1" "$GICOUNT1" ".gitignore has exactly one '/gi.bin' entry after first add"
mkrandfile gi.bin 256   # change content so it re-adds (copied)
dvs add gi.bin >/dev/null
GICOUNT2="$(grep -cxF '/gi.bin' .gitignore || true)"
check "1" "$GICOUNT2" ".gitignore still has exactly one '/gi.bin' entry after re-add (no dup)"
# sidecar itself must NOT be gitignored
SIDECAR_IGNORED="$(git check-ignore .dvs/gi.bin.dvs >/dev/null 2>&1 && echo yes || echo no)"
check "no" "$SIDECAR_IGNORED" ".dvs sidecar is NOT gitignored"

# =====================================================================
say
say "=== add in a repo with NO .git -> succeeds, no .gitignore written (L444) ==="
NOGIT_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
NOGIT_STORAGE="$SCRIPT_DIR/dvs_storage_cli_${NOGIT_REPO##*_}"
mkdir "$NOGIT_STORAGE"
NOGIT_STORAGE_ABS="$(cd "$NOGIT_STORAGE" && pwd)"
(
  cd "$NOGIT_REPO"
  # deliberately NO git init
  dvs init "$NOGIT_STORAGE_ABS" >/dev/null
  mkrandfile ng.bin 128
  rc=0; dvs add ng.bin >/dev/null 2>&1 || rc=$?
  check "0" "$rc" "no-.git repo: add exits 0"
  check "YES" "$([ -e .dvs/ng.bin.dvs ] && echo YES || echo NO)" "no-.git repo: sidecar created"
  check "NO" "$([ -e .gitignore ] && echo YES || echo NO)" "no-.git repo: NO .gitignore written"
)
cd "$DVS_REPO"

# =====================================================================
say
say "=== audit entries: action=add with file{path,hashes.blake3} + compression (L379-419) ==="
mkrandfile audited.bin 128
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
                and a.get("compression") in ("zstd", "none")
                and "operation_id" in e
                and "timestamp" in e
                and "user" in e):
            found = True
            break
print("yes" if found else "no")
PY
)"
check "yes" "$AUDIT_OK" "audit add entry has operation_id/timestamp/user + file{path,hashes.blake3} + compression"

# =====================================================================
say
say "=== symlink to in-project file -> resolved, sidecar at REAL target (L158) ==="
mkrandfile realtarget.bin 128
ln -s realtarget.bin slink.bin
rc=0; OUT="$(dvs add slink.bin 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "in-project symlink add exits 0"
check "yes" "$([ -e .dvs/realtarget.bin.dvs ] && echo yes || echo no)" "symlink resolved -> sidecar at realtarget.bin.dvs"
check "no" "$([ -e .dvs/slink.bin.dvs ] && echo yes || echo no)" "no sidecar created under the link name slink.bin"

# =====================================================================
say
say "=== symlink whose target is OUTSIDE the project root -> rejected (L158) ==="
ln -s "$OUTSIDE_DIR/ext.bin" outside.bin
rc=0; OUT="$(dvs add outside.bin 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "outside-root symlink add exits 1"
OUTREJ="$(printf '%s' "$OUT" | grep -qiE 'outside project|outside the project' && echo yes || echo no)"
check "yes" "$OUTREJ" "outside-root symlink rejected with 'outside project' error"
check "NO" "$([ -e .dvs/outside.bin.dvs ] && echo YES || echo NO)" "no sidecar created for outside-root symlink"

# =====================================================================
say
say "=== stored blob is content-addressed (hash split 2/rest) and read-only after write (L358-368) ==="
mkrandfile blob.bin 128
BHASH="$(dvs add --json blob.bin | python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["hash"])')"
BLOB_PATH="$DVS_STORAGE_ABS/${BHASH:0:2}/${BHASH:2}"
check "YES" "$([ -e "$BLOB_PATH" ] && echo YES || echo NO)" "blob stored at <storage>/<hash[0:2]>/<hash[2:]>"
# read-only: no write bits anywhere (perms like 0440/0444; never 6/7 in any position).
BLOB_PERM="$(stat -f '%Lp' "$BLOB_PATH")"
say "blob perm = $BLOB_PERM"
WRITABLE="$(printf '%s' "$BLOB_PERM" | grep -qE '[2367]' && echo yes || echo no)"
check "no" "$WRITABLE" "stored blob is read-only after write (no write bits, perm=$BLOB_PERM)"

# =====================================================================
say
say "=== atomicity: no .tmp leftovers in storage (L364-366) ==="
TMPCOUNT="$(find "$DVS_STORAGE_ABS" -name '*.tmp' | wc -l | tr -d ' ')"
check "0" "$TMPCOUNT" "no .tmp leftovers in storage after all adds"

# =====================================================================
say
echo "=== SUMMARY: $PASS pass, $FAIL fail ==="

# Self-clean ONLY our own temp dirs (do NOT run cleanup.sh — concurrency).
cd "$SCRIPT_DIR"
rm -rf "$DVS_REPO" "$DVS_STORAGE" "$NOGIT_REPO" "$NOGIT_STORAGE" "$OUTSIDE_DIR"
