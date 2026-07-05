#!/usr/bin/env bash

# Scenario coverage for SCENARIOS.md "cases_overwrite.sh — overwriting, state &
# config transitions". Focuses on TRANSITIONS: content overwrites, identical-byte
# replaces, get-over-unsynced (data-loss), per-file compression across dvs.toml
# flips, mtime-change-with-same-content re-add (metadata-equality / cache), and
# the optional hash cache surviving corruption. CLI only; dvs already installed.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# ── Verdict tracking ──
PASS_COUNT=0
FAIL_COUNT=0
check() {
  { set +x; } 2>/dev/null
  # check WANT GOT LABEL
  if [ "$1" = "$2" ]; then
    echo "PASS: $3"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "FAIL: $3 (want=$1 got=$2)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
  fi
  { set -x; } 2>/dev/null
}

# JSON field extractor (python3 always present; avoid jq).
# json_field FILE_OR_STDIN PYTHON_EXPR   where doc is bound to `d`.
json_get() {
  { set +x; } 2>/dev/null
  python3 -c '
import json,sys
d=json.load(sys.stdin)
print(eval(sys.argv[1]))
' "$1"
  { set -x; } 2>/dev/null
}

# Read the per-file compression recorded in a sidecar.
sidecar_compression() {
  { set +x; } 2>/dev/null
  python3 -c '
import json,sys
print(json.load(open(sys.argv[1]))["compression"])
' "$1"
  { set -x; } 2>/dev/null
}

sidecar_hash() {
  { set +x; } 2>/dev/null
  python3 -c '
import json,sys
print(json.load(open(sys.argv[1]))["hashes"]["blake3"])
' "$1"
  { set -x; } 2>/dev/null
}

# Portable in-place compression flip in dvs.toml.
flip_compression() {
  { set +x; } 2>/dev/null
  python3 - "$1" "$2" "$3" <<'PY'
import sys
p, frm, to = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read().replace('compression = "%s"' % frm, 'compression = "%s"' % to)
open(p, 'w').write(s)
PY
  { set -x; } 2>/dev/null
}

# ── Setup: one default (zstd) repo + sibling storage outside the repo ──
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# ───────────────────────────────────────────────────────────
say
say "=== Overwrite content -> unsynced -> re-add copies new hash (L170) ==="
# ───────────────────────────────────────────────────────────

printf 'original-content\n' > A.bin
dvs add A.bin
SC_A=".dvs/A.bin.dvs"
HASH_BEFORE="$(sidecar_hash "$SC_A")"

say "--- overwrite A's content locally (longer, different bytes) ---"
printf 'modified-content-which-is-longer\n' > A.bin

say "--- status must now report A unsynced ---"
ST_JSON="$(dvs status --json)"
A_STATUS="$(printf '%s' "$ST_JSON" | json_get "[x['status'] for x in d if x['path']=='A.bin'][0]")"
check "unsynced" "$A_STATUS" "overwritten content -> status unsynced (L170)"

say "--- add again -> copied, NEW hash + size recorded ---"
ADD_JSON="$(dvs add --json A.bin)"
A_OUTCOME="$(printf '%s' "$ADD_JSON" | json_get "[x['outcome'] for x in d if x['path']=='A.bin'][0]")"
check "copied" "$A_OUTCOME" "re-add after content change -> outcome copied (L170)"
HASH_AFTER="$(sidecar_hash "$SC_A")"
if [ "$HASH_BEFORE" != "$HASH_AFTER" ]; then NEWHASH=yes; else NEWHASH=no; fi
check "yes" "$NEWHASH" "re-add after content change -> sidecar records NEW hash (L170)"
A_SIZE="$(json_get "[x['size'] for x in d if x['path']=='A.bin'][0]" <<<"$ADD_JSON")"
DISK_A="$(wc -c < A.bin | tr -d ' ')"
check "$DISK_A" "$A_SIZE" "re-add records new size matching disk (L170)"

# ───────────────────────────────────────────────────────────
say
say "=== Identical-byte replace -> present (metadata-equality hash+size, L171) ==="
# ───────────────────────────────────────────────────────────

printf 'stable-payload\n' > B.bin
dvs add B.bin
SC_B=".dvs/B.bin.dvs"
HASH_B1="$(sidecar_hash "$SC_B")"

say "--- replace B with byte-identical content, then re-add ---"
printf 'stable-payload\n' > B.bin
ADD_B_JSON="$(dvs add --json B.bin)"
B_OUTCOME="$(json_get "[x['outcome'] for x in d if x['path']=='B.bin'][0]" <<<"$ADD_B_JSON")"
check "present" "$B_OUTCOME" "byte-identical replace -> outcome present (L171,L354)"
HASH_B2="$(sidecar_hash "$SC_B")"
check "$HASH_B1" "$HASH_B2" "byte-identical replace -> hash unchanged (L354)"

# ───────────────────────────────────────────────────────────
say
say "=== get OVER an unsynced local file: data-loss verdict (L172) ==="
# ───────────────────────────────────────────────────────────

printf 'stored-version\n' > C.bin
dvs add C.bin
SC_C=".dvs/C.bin.dvs"
STORED_HASH_C="$(sidecar_hash "$SC_C")"
STORED_SIZE_C="$(wc -c < C.bin | tr -d ' ')"

say "--- make a LOCAL EDIT (unsynced) then get C.bin ---"
LOCAL_EDIT="locally-edited-content-DIFFERENT-and-longer"
printf '%s\n' "$LOCAL_EDIT" > C.bin
# confirm it is unsynced before get
C_STATUS_PRE="$(dvs status --json | json_get "[x['status'] for x in d if x['path']=='C.bin'][0]")"
check "unsynced" "$C_STATUS_PRE" "local edit before get -> unsynced (precondition, L172)"

GET_RC=0
dvs get C.bin || GET_RC=$?
check "0" "$GET_RC" "get over unsynced file exits 0 (does NOT refuse) (L172)"

# Determine the data-loss verdict by inspecting the resulting bytes.
GOT_CONTENT="$(cat C.bin)"
GOT_SIZE="$(wc -c < C.bin | tr -d ' ')"
say "--- resulting C.bin content after get: $GOT_CONTENT ---"
if [ "$GOT_CONTENT" = "stored-version" ]; then
  GET_VERDICT="overwrote-local"
elif [ "$GOT_CONTENT" = "$LOCAL_EDIT" ]; then
  GET_VERDICT="preserved-local"
else
  GET_VERDICT="other"
fi
check "overwrote-local" "$GET_VERDICT" "get OVERWRITES unsynced local edits (DATA LOSS, L172)"
check "$STORED_SIZE_C" "$GOT_SIZE" "post-get size matches the STORED version, not the edit (L172)"

# ───────────────────────────────────────────────────────────
say
say "=== Per-file compression: zstd, flip toml to none, add NEW file (L173) ==="
# ───────────────────────────────────────────────────────────

# default repo is zstd. Add OLD file under zstd.
printf 'old-zstd-file-content\n' > comp_old.bin
dvs add comp_old.bin
SC_OLD=".dvs/comp_old.bin.dvs"
check "zstd" "$(sidecar_compression "$SC_OLD")" "old file added under default zstd (L173)"

say "--- flip dvs.toml zstd -> none ---"
grep compression dvs.toml || true
flip_compression dvs.toml zstd none
grep compression dvs.toml

say "--- add a NEW file: it must record none ---"
printf 'new-none-file-content\n' > comp_new.bin
dvs add comp_new.bin
SC_NEW=".dvs/comp_new.bin.dvs"
check "none" "$(sidecar_compression "$SC_NEW")" "NEW file after flip -> compression none (L173,L373)"

say "--- old file's sidecar must STILL say zstd (per-file, L173) ---"
check "zstd" "$(sidecar_compression "$SC_OLD")" "OLD file still zstd after toml flip (per-file, L173,L375)"

# ───────────────────────────────────────────────────────────
say
say "=== Per-file compression: none, flip toml to zstd, GET old file (L174,L356-357) ==="
# ───────────────────────────────────────────────────────────

# Use a SEPARATE no-compression repo so the OLD file is genuinely 'none'.
DVS_REPO_NC="$SCRIPT_DIR/dvs_repo_cli_nc_$RUN_SUFFIX"
DVS_STORAGE_NC="$SCRIPT_DIR/dvs_storage_cli_nc_$RUN_SUFFIX"
mkdir -p "$DVS_REPO_NC" "$DVS_STORAGE_NC"
cd "$DVS_REPO_NC"
git init -q
dvs init --no-compression "$DVS_STORAGE_NC"

mkrandfile old_none.bin 4K
dvs add old_none.bin
SC_ON=".dvs/old_none.bin.dvs"
check "none" "$(sidecar_compression "$SC_ON")" "old file added under none (L174)"
ON_HASH="$(sidecar_hash "$SC_ON")"
ON_SIZE="$(wc -c < old_none.bin | tr -d ' ')"

say "--- flip dvs.toml none -> zstd ---"
flip_compression dvs.toml none zstd
grep compression dvs.toml

say "--- delete local, GET the OLD none-compressed file: must still work ---"
rm old_none.bin
GET_OLD_RC=0
dvs get old_none.bin || GET_OLD_RC=$?
check "0" "$GET_OLD_RC" "get of pre-flip none-file succeeds after toml flipped to zstd (L174,L375)"
GOT_ON_SIZE="$(wc -c < old_none.bin | tr -d ' ')"
check "$ON_SIZE" "$GOT_ON_SIZE" "retrieved none-file size matches metadata after flip (L356-357)"
GOT_ON_HASH="$(printf '%s' "$(sidecar_hash "$SC_ON")")"
# the sidecar must be unchanged by get (still none + same hash)
check "none" "$(sidecar_compression "$SC_ON")" "old file sidecar still none after get (per-file, L375)"
check "$ON_HASH" "$GOT_ON_HASH" "old file hash unchanged by get (L375)"

# ───────────────────────────────────────────────────────────
say
say "=== Re-add with changed mtime, identical content -> present (cache mtime+size, L175) ==="
# ───────────────────────────────────────────────────────────

cd "$DVS_REPO"
printf 'mtime-stable-content\n' > mt.bin
dvs add mt.bin
SC_MT=".dvs/mt.bin.dvs"
MT_HASH1="$(sidecar_hash "$SC_MT")"

say "--- bump mtime (touch) WITHOUT changing content, then re-add ---"
# bump far into the future so size matches but mtime differs from cached entry
touch -d '2030-01-01T00:00:00' mt.bin 2>/dev/null \
  || touch -t 203001010000 mt.bin 2>/dev/null \
  || touch mt.bin
ADD_MT_JSON="$(dvs add --json mt.bin)"
MT_OUTCOME="$(json_get "[x['outcome'] for x in d if x['path']=='mt.bin'][0]" <<<"$ADD_MT_JSON")"
check "present" "$MT_OUTCOME" "mtime changed, content identical -> outcome present (L175,L354)"
check "$MT_HASH1" "$(sidecar_hash "$SC_MT")" "mtime-only change -> hash unchanged (L175)"

# ───────────────────────────────────────────────────────────
say
say "=== Hash cache is optional: corrupt dvs.db, status + add still succeed (L176) ==="
# ───────────────────────────────────────────────────────────

[ -f .dvs/.cache/dvs.db ] && CACHE_DB=yes || CACHE_DB=no
check "yes" "$CACHE_DB" "hash cache db exists before corruption (L405,L423)"

say "--- corrupt the cache db ---"
echo garbage > .dvs/.cache/dvs.db
STATUS_RC=0
dvs status || STATUS_RC=$?
check "0" "$STATUS_RC" "status succeeds with corrupt cache (L176,L426)"

printf 'after-corrupt-content\n' > after_corrupt.bin
ADD_CORRUPT_RC=0
dvs add after_corrupt.bin || ADD_CORRUPT_RC=$?
check "0" "$ADD_CORRUPT_RC" "add succeeds with corrupt cache (L176,L426)"

# ───────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS_COUNT pass, $FAIL_COUNT fail ==="
say "=== get-over-unsynced verdict: $GET_VERDICT ==="
# ───────────────────────────────────────────────────────────

# Self-clean ONLY our own dirs (do NOT run ui/cleanup.sh).
say "--- cleaning own temp dirs ---"
cd "$SCRIPT_DIR"
rm -rf "$DVS_REPO" "$DVS_STORAGE" "$DVS_REPO_NC" "$DVS_STORAGE_NC"

[ "$FAIL_COUNT" -eq 0 ]
