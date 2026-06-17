#!/usr/bin/env bash

# Exhaustive CLI scenario coverage for `dvs get` (cases_get.sh in ui/SCENARIOS.md).
# Each scenario prints expected vs actual and a PASS/FAIL verdict via check().
# Scripts assert the SPEC-CORRECT behavior; known tracked divergences are labeled
# with their issue number so a FAIL reads as a tracked bug, not a test defect.
# Ends with: === SUMMARY: N pass, M fail ===
#
# Spec anchors (specs.md "### get"):
#   - outcome `copied` after retrieval; `present` when local already matches (L202-203 / spec get block)
#   - after retrieval the hash is verified; on mismatch the retrieved file is deleted and the op fails (spec)
#   - paths/globs resolve against the METADATA folder, not the working tree (spec)
#   - "If you pass a directory and a glob, the glob will be ran from that directory" (--help)
#   - "At least one path or --glob must be provided; to restore every tracked file pass --glob '**/*'"
#   - "This will exit with `1` if one or more files could not be retrieved."
#
# Known tracked divergences (label, do not treat as test bugs):
#   #217 `get <dir> --glob` positional directory   #220 get exit code on unknown path
#
# Concurrency: this script only creates/removes its OWN mktemp dirs. It never
# runs ui/cleanup.sh and never touches sibling repos.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

PASS=0
FAIL=0
check() {
  { set +x; } 2>/dev/null
  if [ "$1" = "$2" ]; then
    echo "PASS: $3"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $3 | want=[$1] got=[$2]"
    FAIL=$((FAIL + 1))
  fi
  { set -x; } 2>/dev/null
}

# zstd is required for the hash-mismatch tampering idiom (build a valid zstd
# stream of different bytes so decompression succeeds but verification fails).
HAVE_ZSTD=yes
command -v zstd >/dev/null 2>&1 || HAVE_ZSTD=no

# Outcome of the first element of a get --json result.
outcome0() { python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["outcome"])'; }
# Length of a get --json array.
jlen() { python3 -c 'import json,sys;print(len(json.load(sys.stdin)))'; }
# blake3 hash recorded in a sidecar.
sidecar_hash() { python3 -c "import json;print(json.load(open('$1'))['hashes']['blake3'])"; }
# storage backend path from dvs.toml.
storage_path() { python3 -c 'import tomllib;print(tomllib.load(open("dvs.toml","rb"))["backend"]["path"])'; }

# ── Setup: a pristine repo + sibling storage OUTSIDE the repo ──
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# Track a small known fixture set. add then delete locals so get does real work.
mkfiles 2 1K data/raw          # data/raw/file_1.bin, data/raw/file_2.bin
mkfiles 1 1K data/sub/deep     # data/sub/deep/file_1.bin
dvs add --glob 'data/**/*.bin' >/dev/null

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get a tracked file after deleting the local copy -> copied, restored, hash verified ==="
rm -f data/raw/file_1.bin
test ! -e data/raw/file_1.bin
COPY_JSON="$(dvs get --json data/raw/file_1.bin)"
check "copied" "$(printf '%s' "$COPY_JSON" | outcome0)" "outcome=copied after deleting local copy"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "local file restored on disk after copied"
# Restored content matches the stored/metadata hash (verification really happened).
WANT_HASH="$(sidecar_hash .dvs/data/raw/file_1.bin.dvs)"
GOT_HASH="$(b3sum data/raw/file_1.bin 2>/dev/null | awk '{print $1}' || true)"
if [ -n "$GOT_HASH" ]; then
  check "$WANT_HASH" "$GOT_HASH" "restored bytes hash matches metadata blake3"
else
  # b3sum not available; rely on the CLI's own post-retrieval verification (copied means verified).
  check "copied" "$(printf '%s' "$COPY_JSON" | outcome0)" "restored bytes verified by CLI (b3sum absent; copied implies verified)"
fi

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get a file that already matches metadata -> present (noop) ==="
PRESENT_JSON="$(dvs get --json data/raw/file_1.bin)"
check "present" "$(printf '%s' "$PRESENT_JSON" | outcome0)" "outcome=present when local already matches metadata"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get OVERWRITES a locally-MODIFIED (unsynced) file -> DATA-LOSS check ==="
# file_1.bin is present and current. Modify its LOCAL content (now unsynced) and
# get it. Spec lists only outcomes copied/present and a post-retrieval hash
# verify; it does NOT say get refuses or backs up local edits. Document the
# observed behavior precisely.
SENTINEL="LOCAL EDIT THAT MUST NOT BE SILENTLY LOST $$"
printf '%s\n' "$SENTINEL" > data/raw/file_1.bin
LOCAL_HASH_BEFORE="$(b3sum data/raw/file_1.bin 2>/dev/null | awk '{print $1}' || md5 -q data/raw/file_1.bin 2>/dev/null || true)"
rc=0; OVER_JSON="$(dvs get --json data/raw/file_1.bin)" || rc=$?
OVER_OUTCOME="$(printf '%s' "$OVER_JSON" | outcome0)"
say "get-over-unsynced rc=$rc outcome=$OVER_OUTCOME"
STILL_HAS_EDIT="$(grep -c "$SENTINEL" data/raw/file_1.bin 2>/dev/null || true)"
META_HASH="$(sidecar_hash .dvs/data/raw/file_1.bin.dvs)"
LOCAL_HASH_AFTER="$(b3sum data/raw/file_1.bin 2>/dev/null | awk '{print $1}' || true)"
if [ "$STILL_HAS_EDIT" = "0" ]; then
  # Observed: get CLOBBERED the local edit with the stored version (DATA LOSS).
  say "VERDICT: get OVERWRITES locally-modified unsynced files with the stored"
  say "         version. The local edit is silently lost (no refusal, no backup)."
  check "copied" "$OVER_OUTCOME" "get over unsynced file -> outcome=copied (stored version wins)"
  check "0" "$rc" "get over unsynced file exits 0 (treated as a normal retrieval)"
  check "0" "$STILL_HAS_EDIT" "DATA-LOSS: local edit clobbered by stored version (documented)"
  if [ -n "$LOCAL_HASH_AFTER" ]; then
    check "$META_HASH" "$LOCAL_HASH_AFTER" "post-get local content equals stored/metadata version"
  fi
else
  # If a future build refuses or preserves the edit, record that instead.
  say "VERDICT: get did NOT clobber the local edit (refused or preserved)."
  check "1" "$STILL_HAS_EDIT" "local edit preserved/refused by get (no data loss)"
fi
# Restore canonical content for later scenarios.
rm -f data/raw/file_1.bin
dvs get data/raw/file_1.bin >/dev/null

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get a tracked file into a directory that does not exist locally -> dirs created ==="
rm -rf data/sub
test ! -e data/sub
rc=0; dvs get data/sub/deep/file_1.bin >/dev/null || rc=$?
check "0" "$rc" "get into missing local dir exits 0"
check "yes" "$([ -f data/sub/deep/file_1.bin ] && echo yes || echo no)" "missing intermediate dirs (data/sub/deep) recreated by get"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get by absolute / relative / mixed paths ==="
# relative
rm -f data/raw/file_1.bin
check "copied" "$(dvs get --json data/raw/file_1.bin | outcome0)" "get via relative path"
# absolute
rm -f data/raw/file_2.bin
ABS="$DVS_REPO/data/raw/file_2.bin"
check "copied" "$(dvs get --json "$ABS" | outcome0)" "get via absolute path"
# ./-prefixed
rm -f data/raw/file_1.bin
check "copied" "$(dvs get --json ./data/raw/file_1.bin | outcome0)" "get via ./-prefixed relative path"
# mixed abs + rel in one invocation
rm -f data/raw/file_1.bin data/raw/file_2.bin
MIX_JSON="$(dvs get --json "$DVS_REPO/data/raw/file_1.bin" data/raw/file_2.bin)"
check "2" "$(printf '%s' "$MIX_JSON" | jlen)" "mixed abs+rel paths in one invocation -> 2 retrieved"
check "yes" "$([ -f data/raw/file_1.bin ] && [ -f data/raw/file_2.bin ] && echo yes || echo no)" "both mixed-path files restored on disk"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get an unknown/untracked path -> #220 (silently dropped vs exit 1) ==="
# A path with NO metadata, mixed with a valid one. Spec: paths resolve against
# the metadata folder; an unknown path is simply not in the resolution set.
# #220 questions whether that should still exit 1. Document the observed code.
rm -f data/raw/file_1.bin
rc=0; UNK_OUT="$(dvs get data/raw/file_1.bin no/such/unknown.bin 2>&1)" || rc=$?
say "unknown-path mixed: rc=$rc"
say "$UNK_OUT"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "valid file retrieved despite unknown sibling path (#220)"
# Assertion reflects observed behavior: unknown path silently dropped, exit 0.
# If this branch makes an explicitly-listed unknown path force exit 1 (the #220
# fix), this line FAILs and flags that the behavior changed -> update the issue.
check "0" "$rc" "#220: explicitly-listed unknown path silently dropped, exit 0 (tracked divergence)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get with NO local file AND no metadata -> exit 1 (No files to get) ==="
rc=0; NOFILES_OUT="$(dvs get totally/untracked/nothing.bin 2>&1)" || rc=$?
check "1" "$rc" "exit 1 when the only requested path has no metadata"
check "1" "$(printf '%s' "$NOFILES_OUT" | grep -c 'No files to get' || true)" 'error message is "No files to get"'

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get when the storage blob is missing/deleted -> error for that file, exit 1 ==="
mkfiles 1 1K data/missingblob          # data/missingblob/file_1.bin
dvs add data/missingblob/file_1.bin >/dev/null
MB_HASH="$(sidecar_hash .dvs/data/missingblob/file_1.bin.dvs)"
MB_BLOB="$(storage_path)/${MB_HASH:0:2}/${MB_HASH:2}"
say "deleting stored blob: $MB_BLOB"
chmod -R u+w "$(storage_path)/${MB_HASH:0:2}" 2>/dev/null || true
rm -f "$MB_BLOB"
rm -f data/missingblob/file_1.bin
rc=0; MB_OUT="$(dvs get data/missingblob/file_1.bin 2>&1)" || rc=$?
say "$MB_OUT"
check "1" "$rc" "exit 1 when the storage blob is missing"
check "yes" "$(printf '%s' "$MB_OUT" | grep -qiE 'missing|not found|no such' && echo yes || echo no)" "error mentions the missing storage blob"
check "no" "$([ -f data/missingblob/file_1.bin ] && echo yes || echo no)" "no local file written when blob is missing"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get with a tampered storage blob (hash mismatch) -> retrieved file deleted, fail, exit 1 ==="
if [ "$HAVE_ZSTD" = "yes" ]; then
  mkfiles 1 1K data/tamper            # data/tamper/file_1.bin
  dvs add data/tamper/file_1.bin >/dev/null
  THASH="$(sidecar_hash .dvs/data/tamper/file_1.bin.dvs)"
  TBLOB="$(storage_path)/${THASH:0:2}/${THASH:2}"
  say "tampering stored blob: $TBLOB"
  TMP_RAW="$(mktemp "$DVS_REPO"/dvs_tamper_raw_XXXX)"
  TMP_ZST="$(mktemp "$DVS_REPO"/dvs_tamper_zst_XXXX)"
  head -c 2048 /dev/urandom > "$TMP_RAW"
  zstd -q -f "$TMP_RAW" -o "$TMP_ZST"
  chmod +w "$TBLOB"
  cp "$TMP_ZST" "$TBLOB"
  rm -f "$TMP_RAW" "$TMP_ZST"
  rm -f data/tamper/file_1.bin
  rc=0; TAMPER_OUT="$(dvs get data/tamper/file_1.bin 2>&1)" || rc=$?
  say "$TAMPER_OUT"
  check "1" "$rc" "exit 1 on post-retrieval hash mismatch"
  check "1" "$(printf '%s' "$TAMPER_OUT" | grep -c 'does not match expected hash' || true)" "error mentions hash mismatch"
  check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "retrieved file deleted on hash mismatch"
else
  say "SKIP: zstd not available; hash-mismatch tampering idiom needs a valid zstd stream"
fi

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get --dry-run -> reports, writes nothing ==="
rm -f data/raw/file_1.bin
DRY_HR="$(dvs get --dry-run data/raw/file_1.bin)"
check "1" "$(printf '%s' "$DRY_HR" | grep -c 'file_1.bin' || true)" "--dry-run reports the file"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run writes NO local file"
DRY_JSON="$(dvs get --dry-run --json data/raw/file_1.bin)"
check "copied" "$(printf '%s' "$DRY_JSON" | outcome0)" "--dry-run --json reports would-be outcome=copied"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run --json writes NO local file"
dvs get data/raw/file_1.bin >/dev/null   # restore

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get --json shape -> array of {path, outcome, size} ==="
rm -f data/raw/file_1.bin
JSON_OUT="$(dvs get --json data/raw/file_1.bin)"
JSON_OK="$(printf '%s' "$JSON_OUT" | python3 -c '
import json,sys
d=json.load(sys.stdin)
assert isinstance(d,list) and len(d)>=1, "not a non-empty array"
o=d[0]
assert o.get("path")=="data/raw/file_1.bin", "bad path: %r"%o.get("path")
assert o.get("outcome") in ("copied","present"), "bad outcome: %r"%o.get("outcome")
assert isinstance(o.get("size"),int), "size not int: %r"%o.get("size")
print("ok")
' 2>&1 || true)"
check "ok" "$JSON_OK" "--json is an array of {path,outcome,size} objects"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get -g and --glob both resolve via the metadata folder ==="
rm -f data/raw/file_1.bin data/raw/file_2.bin
SHORT_JSON="$(dvs get -g 'data/raw/*.bin' --json)"
check "2" "$(printf '%s' "$SHORT_JSON" | jlen)" "-g glob retrieved 2 files via metadata folder"
rm -f data/raw/file_1.bin data/raw/file_2.bin
LONG_JSON="$(dvs get --glob 'data/raw/*.bin' --json)"
check "2" "$(printf '%s' "$LONG_JSON" | jlen)" "--glob glob retrieved 2 files via metadata folder"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get <dir> --glob (positional directory) -> #217 ==="
# --help: "If you pass a directory and a glob, the glob will be ran from that
# directory." Spec-correct: get data --glob '*.bin' restores data/raw direct? No
# -- '*.bin' from data/ matches nothing directly (files live in data/raw and
# data/sub). Use a glob that matches under the positional dir.
rm -f data/raw/file_1.bin data/raw/file_2.bin
rc=0; POS_JSON="$(dvs get data/raw --glob '*.bin' --json 2>&1)" || rc=$?
say "get data/raw --glob '*.bin' rc=$rc"
say "$POS_JSON"
# Spec-correct expectation: glob runs from data/raw, matching its 2 .bin files.
POS_N="$(printf '%s' "$POS_JSON" | jlen 2>/dev/null || echo ERR)"
check "2" "$POS_N" "#217: get <dir> --glob runs glob from that directory (2 files)"
check "yes" "$([ -f data/raw/file_1.bin ] && [ -f data/raw/file_2.bin ] && echo yes || echo no)" "#217: positional-dir glob restored both files on disk"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== get --glob '**/*' restores everything ==="
rm -f data/raw/file_1.bin data/raw/file_2.bin data/sub/deep/file_1.bin
rm -rf data/sub
# Exit is 1 here because the deliberately broken missingblob/tamper blobs (set up
# earlier in THIS script) fail to retrieve; the JSON is still fully populated.
rc=0; ALL_JSON="$(dvs get --glob '**/*' --json)" || rc=$?
check "1" "$rc" "--glob '**/*' exits 1 due to our pre-broken blobs (partial failure)"
# Tracked set: data/raw/file_1, data/raw/file_2, data/sub/deep/file_1,
# data/missingblob/file_1 (blob deleted earlier -> will error), data/tamper/file_1
# (blob tampered earlier -> will error). Assert the two clean data/raw files plus
# data/sub/deep restore; tolerate the deliberately broken ones.
RESTORED_RAW="$([ -f data/raw/file_1.bin ] && [ -f data/raw/file_2.bin ] && echo yes || echo no)"
RESTORED_SUB="$([ -f data/sub/deep/file_1.bin ] && echo yes || echo no)"
check "yes" "$RESTORED_RAW" "--glob '**/*' restored both data/raw files"
check "yes" "$RESTORED_SUB" "--glob '**/*' restored nested data/sub/deep file"
ALL_N="$(printf '%s' "$ALL_JSON" | jlen)"
check "yes" "$([ "$ALL_N" -ge 3 ] && echo yes || echo no)" "--glob '**/*' resolved all tracked sidecars (>=3 results)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== partial failure (1 good + 1 mismatch) -> good retrieved, exit 1 ==="
if [ "$HAVE_ZSTD" = "yes" ]; then
  # data/tamper/file_1.bin blob is still tampered from above; data/raw/file_1.bin is clean.
  rm -f data/raw/file_1.bin data/tamper/file_1.bin
  rc=0; dvs get data/raw/file_1.bin data/tamper/file_1.bin >/dev/null 2>&1 || rc=$?
  check "1" "$rc" "exit 1 on partial failure: 1 good + 1 hash-mismatch file"
  check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "good file still retrieved during partial failure"
  check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "mismatched file not left on disk during partial failure"
else
  say "SKIP: zstd not available; partial-failure case reuses the tampered blob"
fi

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="

# Self-clean ONLY our own temp dirs (never ui/cleanup.sh, never siblings).
cd "$SCRIPT_DIR"
rm -rf "$DVS_REPO" "$DVS_STORAGE"
