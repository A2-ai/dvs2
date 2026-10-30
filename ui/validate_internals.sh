#!/usr/bin/env bash

# Validate CLI-observable claims from specs.md "Internals" sections:
#   Metadata file format (L670), Storage layout (L699),
#   Compression (L713), Hash cache (L765).
# Line refs (Lnnn) are to specs.md on main at 7ff0aa0.
# Observes internals purely by running `dvs` and inspecting on-disk files.

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

# ── Setup: a default (zstd) repo + a no-compression repo ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init local "$DVS_STORAGE"

# ───────────────────────────────────────────────────────────
say
say "=== Metadata file format (specs L670-697) ==="
# ───────────────────────────────────────────────────────────

mkrandfile data/input.bin 1K
dvs add -m "first add" data/input.bin

SIDECAR=".dvs/data/input.bin.dvs"

# sidecar created at <metadata>/<path>.dvs mirroring the file path (L672-673)
say "--- sidecar path mirrors project layout ---"
[ -f "$SIDECAR" ] && SC_EXISTS=yes || SC_EXISTS=no
check yes "$SC_EXISTS" "metadata sidecar at .dvs/data/input.bin.dvs (L672)"

say "--- sidecar contents ---"
cat "$SIDECAR"

# hashes.blake3 is 64 hex chars (L680, L690)
BLAKE3="$(jq -r '.hashes.blake3' "$SIDECAR")"
if printf '%s' "$BLAKE3" | grep -Eq '^[0-9a-f]{64}$'; then HEXOK=yes; else HEXOK=no; fi
check yes "$HEXOK" "hashes.blake3 is 64 hex chars (L680,L690)"

# size matches file bytes on disk (L682, L691)
META_SIZE="$(jq -r '.size' "$SIDECAR")"
DISK_SIZE="$(wc -c < data/input.bin | tr -d ' ')"
check "$DISK_SIZE" "$META_SIZE" "size matches file bytes (L682,L691)"

# created_by present and non-empty (L683, L692)
CREATED_BY="$(jq -r '.created_by' "$SIDECAR")"
if [ -n "$CREATED_BY" ] && [ "$CREATED_BY" != "null" ]; then CBOK=yes; else CBOK=no; fi
check yes "$CBOK" "created_by present (L683,L692)"

# add_time parses as ISO 8601 (L684, L693)
ADD_TIME="$(jq -r '.add_time' "$SIDECAR")"
if python3 -c "import sys,datetime; datetime.datetime.fromisoformat(sys.argv[1].replace('Z','+00:00'))" "$ADD_TIME" 2>/dev/null; then
  ISOOK=yes
else
  ISOOK=no
fi
check yes "$ISOOK" "add_time parses as ISO 8601 (L684,L693)"

# compression defaults to zstd (L685, L694)
COMP="$(jq -r '.compression' "$SIDECAR")"
check "zstd" "$COMP" "compression defaults to zstd (L685,L694)"

# message present when -m given (L686, L695)
MSG="$(jq -r '.message' "$SIDECAR")"
check "first add" "$MSG" "message recorded when -m given (L686)"

say "--- add WITHOUT -m: message key must be ABSENT (L695) ---"
mkrandfile data/nomsg.bin 1K
dvs add data/nomsg.bin
NOMSG_SC=".dvs/data/nomsg.bin.dvs"
cat "$NOMSG_SC"
# `has("message")` is false when the key is absent
HAS_MSG="$(jq -r 'has("message")' "$NOMSG_SC")"
check "false" "$HAS_MSG" "message key omitted when -m not given (L695)"

# ───────────────────────────────────────────────────────────
say
say "=== Storage layout (specs L699-711) ==="
# ───────────────────────────────────────────────────────────

# content-addressable: blob at <storage>/<first2>/<remaining62> (L701-705)
PREFIX="${BLAKE3:0:2}"
REST="${BLAKE3:2}"
BLOB="$DVS_STORAGE/$PREFIX/$REST"
say "--- expecting blob at $PREFIX/$REST ---"
[ -f "$BLOB" ] && BLOBOK=yes || BLOBOK=no
check yes "$BLOBOK" "blob at <storage>/<2-prefix>/<62-rest> (L701-705)"

# dedup: two DIFFERENT-named files, IDENTICAL content -> ONE blob (L701).
# Added in separate invocations. The single-invocation case (#216, fixed by
# #274) is guarded by audit_violations.sh.
say "--- dedup: two identically-content files share one blob (L701) ---"
DEDUP_CONTENT="dedup-identical-content-payload"
printf '%s\n' "$DEDUP_CONTENT" > dedup_a.bin
printf '%s\n' "$DEDUP_CONTENT" > dedup_b.bin
dvs add dedup_a.bin
dvs add dedup_b.bin
HA="$(jq -r '.hashes.blake3' .dvs/dedup_a.bin.dvs)"
HB="$(jq -r '.hashes.blake3' .dvs/dedup_b.bin.dvs)"
check "$HA" "$HB" "identical content -> identical blake3 (L701)"
DEDUP_BLOBS="$(find "$DVS_STORAGE/${HA:0:2}" -type f -name "${HA:2}" | wc -l | tr -d ' ')"
check "1" "$DEDUP_BLOBS" "identical content -> exactly one storage blob (L701)"

# no .tmp leftovers in storage after add (L707-709)
say "--- no *.tmp leftover in storage (L707-709) ---"
TMP_LEFT="$(find "$DVS_STORAGE" -name '*.tmp' | wc -l | tr -d ' ')"
check "0" "$TMP_LEFT" "no .tmp files left in storage after add (L707-709)"

# stored blobs are read-only (L711): no write bits anywhere
say "--- stored blob permissions (L711) ---"
ls -l "$BLOB"
PERMS="$(ls -l "$BLOB" | awk '{print $1}')"
if printf '%s' "$PERMS" | grep -q 'w'; then WRITABLE=yes; else WRITABLE=no; fi
check no "$WRITABLE" "stored blob has no write bits / read-only (L711)"

# ───────────────────────────────────────────────────────────
say
say "=== Compression (specs L713-718) ==="
# ───────────────────────────────────────────────────────────

# default add already recorded zstd above. Now a SEPARATE --no-compression repo.
DVS_REPO_NC="$SCRIPT_DIR/dvs_repo_cli_nc_$RUN_SUFFIX"
DVS_STORAGE_NC="$SCRIPT_DIR/dvs_storage_cli_nc_$RUN_SUFFIX"
mkdir -p "$DVS_REPO_NC" "$DVS_STORAGE_NC"
cd "$DVS_REPO_NC"
git init -q
dvs init local --no-compression "$DVS_STORAGE_NC"

mkrandfile payload.bin 4K
dvs add payload.bin
NC_SC=".dvs/payload.bin.dvs"
cat "$NC_SC"
NC_COMP="$(jq -r '.compression' "$NC_SC")"
check "none" "$NC_COMP" "--no-compression -> compression none in metadata (L715-716)"

# Changing dvs.toml compression AFTER add must not break get of the
# earlier file: get reads compression from per-file metadata (L716-718).
say "--- flip dvs.toml none -> zstd, then get the none-compressed file (L716-718) ---"
cat dvs.toml
# portable in-place edit
python3 - "$PWD/dvs.toml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read().replace('compression = "none"', 'compression = "zstd"')
open(p, 'w').write(s)
PY
grep compression dvs.toml
# capture the original bytes, delete local, get from storage
ORIG_HASH="$(jq -r '.hashes.blake3' "$NC_SC")"
ORIG_SIZE="$(jq -r '.size' "$NC_SC")"
rm payload.bin
GET_RC=0
dvs get payload.bin || GET_RC=$?
check "0" "$GET_RC" "get of pre-flip none-file succeeds after toml flipped to zstd (L716-718)"
GOT_SIZE="$(wc -c < payload.bin | tr -d ' ')"
check "$ORIG_SIZE" "$GOT_SIZE" "retrieved file size matches metadata after compression flip (L716-718)"

# ───────────────────────────────────────────────────────────
say
say "=== Hash cache (specs L765-774) ==="
# ───────────────────────────────────────────────────────────

cd "$DVS_REPO"

# cache at {metadata}/.cache/dvs.db after add (L767)
say "--- cache db location (L767) ---"
find .dvs/.cache -maxdepth 1 -type f
[ -f .dvs/.cache/dvs.db ] && CACHE_DB=yes || CACHE_DB=no
check yes "$CACHE_DB" "hash cache at .dvs/.cache/dvs.db after add (L767)"

# .cache added to .gitignore in the metadata folder (L769-770)
say "--- .cache gitignored in metadata folder (L769-770) ---"
cat .dvs/.gitignore
if grep -Eq '(^|/)\.cache' .dvs/.gitignore; then CACHE_IGN=yes; else CACHE_IGN=no; fi
check yes "$CACHE_IGN" ".cache dir added to .gitignore (L769-770)"

# cache is optional: corrupt dvs.db, operations still succeed (L770)
say "--- corrupt dvs.db, then status + add must still succeed (L770) ---"
echo garbage > .dvs/.cache/dvs.db
STATUS_RC=0
dvs status || STATUS_RC=$?
check "0" "$STATUS_RC" "status succeeds with corrupt cache (L770)"
mkrandfile data/after_corrupt.bin 1K
ADD_RC=0
dvs add data/after_corrupt.bin || ADD_RC=$?
check "0" "$ADD_RC" "add succeeds with corrupt cache (L770)"

# ───────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS_COUNT pass, $FAIL_COUNT fail ==="
# ───────────────────────────────────────────────────────────

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
