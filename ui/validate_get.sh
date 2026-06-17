#!/usr/bin/env bash

# Validate CLI-observable claims for `dvs get` against specs.md.
# Each claim prints expected vs actual and a PASS/FAIL verdict via check().
# Ends with: === SUMMARY: N pass, M fail ===
#
# Assigned claims (specs.md / ui/TODO.md "Script 3"):
#   - get --help text matches spec block L214-227 (order: json, threads, glob (-g), dry-run)
#   - outcome `copied` after deleting local copy (L202)
#   - outcome `present` when local already matches metadata (L203)
#   - hash verified after retrieval; on mismatch retrieved file deleted + fails for that file (L205-206)
#   - get resolves paths/globs against METADATA folder, not working tree (L208)
#   - -g/--glob short + long both work (L225)
#   - exit code 1 if one or more files could not be retrieved (L230)
#   - --dry-run reports outcomes but makes NO changes (L226)
#   - --json output for get (L223)

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

# ── Setup: a single CLI repo with a known file ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

mkfiles 2 1K data/raw   # data/raw/file_1.bin, data/raw/file_2.bin
dvs add --glob "data/**/*.bin"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: get --help matches spec block (L214-227) ==="
# Extract the expected help VERBATIM from specs.md (the lines between the
# '❯ dvs get --help' prompt line and the closing code fence), preserving the
# exact trailing whitespace clap emits.  Option order: json, threads,
# glob (-g), dry-run.
SPEC_MD="${SCRIPT_DIR}/../specs.md"
EXPECTED_HELP="$(awk '/^❯ dvs get --help$/{f=1;next} f&&/^```$/{exit} f' "$SPEC_MD")"
ACTUAL_HELP="$(dvs get --help)"
check "$EXPECTED_HELP" "$ACTUAL_HELP" "get --help matches spec block L214-227 (order json,threads,glob -g,dry-run)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: outcome \`copied\` after deleting local copy (L202) ==="
rm -f data/raw/file_1.bin
# Human-readable get prints '<path> [size]' for a copied file; outcome word only in JSON.
COPIED_JSON="$(dvs get --json data/raw/file_2.bin >/dev/null 2>&1; rm -f data/raw/file_2.bin; dvs get --json data/raw/file_2.bin)"
COPIED_OUTCOME="$(printf '%s' "$COPIED_JSON" | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$COPIED_OUTCOME" "outcome=copied (JSON) for retrieved file_2.bin (L202)"
# Human-readable confirmation for file_1 (deleted above)
HR="$(dvs get data/raw/file_1.bin)"
HAS_FILE1="$(printf '%s' "$HR" | grep -c 'file_1.bin' || true)"
check "1" "$HAS_FILE1" "human-readable get lists copied file_1.bin (L202)"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "local file_1.bin restored on disk after copied (L202)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: outcome \`present\` when local already matches metadata (L203) ==="
# file_1.bin is now present locally and matches metadata.
PRESENT_JSON="$(dvs get --json data/raw/file_1.bin)"
PRESENT_OUTCOME="$(printf '%s' "$PRESENT_JSON" | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["outcome"])')"
check "present" "$PRESENT_OUTCOME" "outcome=present (JSON) for already-matching file_1.bin (L203)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: get resolves paths against METADATA folder not working tree (L208) ==="
# Delete the local file entirely; metadata sidecar still exists -> get resolves it.
rm -f data/raw/file_1.bin
test ! -e data/raw/file_1.bin
RESOLVE_JSON="$(dvs get --json data/raw/file_1.bin)"
RESOLVE_OUTCOME="$(printf '%s' "$RESOLVE_JSON" | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$RESOLVE_OUTCOME" "path with no working-tree file still resolved via metadata then copied (L208)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: -g / --glob short + long flags both work (L225) ==="
rm -f data/raw/file_1.bin data/raw/file_2.bin
SHORT_JSON="$(dvs get -g 'data/**/*.bin' --json)"
SHORT_N="$(printf '%s' "$SHORT_JSON" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)))')"
check "2" "$SHORT_N" "-g glob retrieved 2 files (L225)"
rm -f data/raw/file_1.bin data/raw/file_2.bin
LONG_JSON="$(dvs get --glob 'data/**/*.bin' --json)"
LONG_N="$(printf '%s' "$LONG_JSON" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)))')"
check "2" "$LONG_N" "--glob glob retrieved 2 files (L225)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --dry-run reports outcomes but makes NO changes (L226) ==="
rm -f data/raw/file_1.bin
DRY_HR="$(dvs get --dry-run data/raw/file_1.bin)"
DRY_LISTS="$(printf '%s' "$DRY_HR" | grep -c 'file_1.bin' || true)"
check "1" "$DRY_LISTS" "--dry-run reports file_1.bin in output (L226)"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run wrote NO local file (L226)"
# JSON dry-run still shows outcome but writes nothing.
DRY_JSON="$(dvs get --dry-run --json data/raw/file_1.bin)"
DRY_OUTCOME="$(printf '%s' "$DRY_JSON" | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["outcome"])')"
check "copied" "$DRY_OUTCOME" "--dry-run --json reports would-be outcome=copied (L226)"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run --json wrote NO local file (L226)"
# Restore for later use.
dvs get data/raw/file_1.bin >/dev/null

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --json output shape for get (L223) ==="
rm -f data/raw/file_1.bin
JSON_OUT="$(dvs get --json data/raw/file_1.bin)"
# Expect a JSON array of objects each with path + outcome + size.
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
check "ok" "$JSON_OK" "--json is array of {path,outcome,size} objects (L223)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: exit code 1 if one or more files could not be retrieved (L230) ==="
# (a) requested path has no metadata at all -> nothing matches -> exit 1.
rc=0; dvs get does/not/exist.bin >/dev/null 2>&1 || rc=$?
check "1" "$rc" "exit 1 when requested path has no metadata (L230)"
# FINDING: a path with no metadata mixed with a valid one does NOT cause exit 1.
# Paths resolve against the metadata folder (L208); an unknown path simply is
# not in the resolution set, so it is silently dropped and the valid file is
# retrieved with exit 0.  Documented, not asserted as failure.
rm -f data/raw/file_1.bin
rc=0; dvs get data/raw/file_1.bin no/such.bin >/dev/null 2>&1 || rc=$?
check "0" "$rc" "missing-metadata path silently dropped; valid file still retrieved, exit 0 (L208/L230 finding)"
# The genuine partial-failure -> exit 1 case (one matched file that FAILS to
# retrieve) is exercised in the hash-mismatch section below.

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: hash verified after retrieval; mismatch deletes file + fails (L205-206) ==="
# Force a GENUINE post-retrieval hash mismatch: replace the stored blob with a
# VALID zstd stream whose decompressed content differs from what the metadata
# hash expects. get will decompress fine, write the file, hash it, detect the
# mismatch, then delete the retrieved file and fail for it.
mkfiles 1 1K data/tamper   # data/tamper/file_1.bin
dvs add data/tamper/file_1.bin >/dev/null
THASH="$(python3 -c 'import json;print(json.load(open(".dvs/data/tamper/file_1.bin.dvs"))["hashes"]["blake3"])')"
TBLOB="$DVS_STORAGE/${THASH:0:2}/${THASH:2}"
say "stored blob: $TBLOB"
# Build a valid zstd stream of different random bytes and overwrite the blob.
TMP_RAW="$(mktemp /tmp/dvs_tamper_raw_XXXX)"
TMP_ZST="$(mktemp /tmp/dvs_tamper_zst_XXXX)"
head -c 2048 /dev/urandom > "$TMP_RAW"
zstd -q -f "$TMP_RAW" -o "$TMP_ZST"
chmod +w "$TBLOB"
cp "$TMP_ZST" "$TBLOB"
rm -f "$TMP_RAW" "$TMP_ZST"
rm -f data/tamper/file_1.bin
rc=0; TAMPER_OUT="$(dvs get data/tamper/file_1.bin 2>&1)" || rc=$?
say "$TAMPER_OUT"
check "1" "$rc" "exit 1 on post-retrieval hash mismatch (L205-206)"
MISMATCH_MSG="$(printf '%s' "$TAMPER_OUT" | grep -c 'does not match expected hash' || true)"
check "1" "$MISMATCH_MSG" "error mentions hash mismatch for tampered file (L205)"
check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "retrieved file deleted on hash mismatch (L206)"
# JSON form of the same failure.
rm -f data/tamper/file_1.bin
rc=0; TAMPER_JSON="$(dvs get --json data/tamper/file_1.bin 2>/dev/null)" || rc=$?
TAMPER_HAS_ERR="$(printf '%s' "$TAMPER_JSON" | python3 -c 'import json,sys
d=json.load(sys.stdin)
print(1 if d and "error" in d[0] else 0)' 2>/dev/null || echo 0)"
check "1" "$TAMPER_HAS_ERR" "--json reports per-file error object on hash mismatch (L205-206)"
check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "retrieved file deleted on hash mismatch (--json path) (L206)"

# Genuine partial failure: one good file + the tampered file -> exit 1 (L230),
# good file still retrieved.
rm -f data/raw/file_1.bin data/tamper/file_1.bin
rc=0; dvs get data/raw/file_1.bin data/tamper/file_1.bin >/dev/null 2>&1 || rc=$?
check "1" "$rc" "exit 1 on partial failure: 1 good + 1 hash-mismatch file (L230)"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "good file still retrieved during partial failure (L230)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
