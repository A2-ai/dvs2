#!/usr/bin/env bash
#
# Showcase: re-adding an UNCHANGED file applies new settings (compression, message).
#
# Every case here was verified broken on pre-fix main (2026-07-16):
#
#   A. Changing `compression` in dvs.toml and re-adding an unchanged file was
#      a silent no-op. The result said `present`, the stored blob stayed in
#      the old encoding, and the sidecar kept the old compression field.
#   B. `dvs add --message` on an unchanged file dropped the message. The
#      sidecar was never rewritten.
#
# With the fix, a settings change on unchanged content is a real re-add:
# a compression change re-stores the blob and rewrites the sidecar, a new
# message rewrites the sidecar only. Both report `copied`. A re-add without
# a message keeps the existing message. A re-add with identical settings
# stays `present` with a byte-identical sidecar, so bulk re-adds do not
# dirty .dvs files in git.
#
# Shared-blob caveat: blobs are keyed by content hash, so two identical files
# share one blob while compression is recorded per sidecar. Re-encoding via
# one file strands the other sidecars. That hazard is documented on PR #264
# and deliberately not asserted here.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-cli\` should have been run first so the dvs CLI reflects this branch."

# region: SETUP

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_readd_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_readd_$RUN_SUFFIX"
SCRATCH="$SCRIPT_DIR/dvs_fixture_readd_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI" "$SCRATCH"

# Rewrite the compression key in dvs.toml (the only supported way to change it).
set_compression() { # $1 = none|zstd
  perl -pi -e "s/^compression = .*/compression = \"$1\"/" dvs.toml
}

# Assert that two files differ. A bare `! cmp` would not trip `set -e`.
assert_differs() { # $1 $2 = files
  if cmp -s "$1" "$2"; then
    say "FAIL: expected $1 and $2 to differ"
    exit 1
  fi
}

cd "$DVS_REPO_CLI"
git init -q .
dvs init "$DVS_STORAGE_CLI" --no-compression

printf 'id,value\n1,uncompressed payload for the re-add walkthrough\n' > data.csv
cp data.csv "$SCRATCH/original.csv"
dvs --json add data.csv | grep '"outcome":"copied"'

HASH="$(grep -o '"blake3": "[a-f0-9]*"' .dvs/data.csv.dvs | cut -d'"' -f4)"
BLOB="$DVS_STORAGE_CLI/${HASH:0:2}/${HASH:2}"
test -f "$BLOB"
grep '"compression": "none"' .dvs/data.csv.dvs
# With compression none the blob is the raw file.
cmp data.csv "$BLOB"
cp "$BLOB" "$SCRATCH/blob_v1"
say "OK setup: tracked uncompressed, blob is the raw bytes"

# region: A — compression change re-encodes the blob

say
say "============================================================"
say "= A: flip dvs.toml to zstd, re-add the unchanged file      ="
say "============================================================"
# Pre-fix: outcome was `present`, blob and sidecar untouched.
set_compression zstd
dvs --json add data.csv | grep '"outcome":"copied"'
grep '"compression": "zstd"' .dvs/data.csv.dvs
# The stored bytes really changed encoding.
assert_differs "$SCRATCH/blob_v1" "$BLOB"
cp "$BLOB" "$SCRATCH/blob_v2"
say "OK: re-add re-encoded the blob and updated the sidecar"

say
say "--- get round-trips through the new encoding ---"
rm data.csv
dvs get data.csv
cmp data.csv "$SCRATCH/original.csv"
say "OK: sidecar says zstd and the blob really is zstd"

# region: B — new message rewrites the sidecar only

say
say "============================================================"
say "= B: re-add with --message updates the sidecar, not blob   ="
say "============================================================"
# Pre-fix: the message was silently dropped.
dvs --json add data.csv --message "quarterly refresh" | grep '"outcome":"copied"'
grep '"message": "quarterly refresh"' .dvs/data.csv.dvs
cmp "$SCRATCH/blob_v2" "$BLOB"
say "OK: message recorded, blob untouched"

# region: C — no message never clears an existing one

say
say "============================================================"
say "= C: plain re-add keeps the message, zero sidecar churn    ="
say "============================================================"
cp .dvs/data.csv.dvs "$SCRATCH/sidecar_v1"
dvs --json add data.csv | grep '"outcome":"present"'
cmp .dvs/data.csv.dvs "$SCRATCH/sidecar_v1"
grep '"message": "quarterly refresh"' .dvs/data.csv.dvs
say "OK: present, sidecar byte-identical, message preserved"

# region: D — re-adding the same settings is also a no-op

say
say "============================================================"
say "= D: same message and compression again stays present      ="
say "============================================================"
dvs --json add data.csv --message "quarterly refresh" | grep '"outcome":"present"'
cmp .dvs/data.csv.dvs "$SCRATCH/sidecar_v1"
say "OK: identical settings do not rewrite anything"

say
say "All re-add settings cases passed."
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
