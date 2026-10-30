#!/usr/bin/env bash

# Validate CLI-observable claims for `dvs get` against specs.md.
# Each claim prints expected vs actual and a PASS/FAIL verdict via check().
# Ends with: === SUMMARY: N pass, M fail ===
# Line refs (Lnnn) are to specs.md on main at 370f448.
#
# Assigned claims (specs.md / ui/TODO.md "Script 3"):
#   - get --help text matches spec block L504-524
#   - a path or --glob is required (L505, L526)
#   - action `restored` after deleting local copy (L461, L556)
#   - action `unchanged` when local already matches metadata (L462, L556)
#   - downloaded bytes verified before install; on mismatch the row fails with
#     hash_mismatch and no file is installed (L478-480, L559)
#   - get resolves paths/globs against the METADATA folder, not working tree (L42, L466)
#   - -g/--glob short + long both work (L513)
#   - exit code 1 if one or more files could not be retrieved (L568-569)
#   - local changes are a conflict: abort writes nothing and exits 3,
#     --force replaces them (L464, L472, L517, L569)
#   - --dry-run reports outcomes but makes NO changes (L44, L515)
#   - --json output for get (L538-566)

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# The on_conflict checks expect the default policy, so the environment must
# not set one (precedence: flag > DVS_GET_ON_CONFLICT > dvs.toml > abort, L491).
unset DVS_GET_ON_CONFLICT

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

# Run a python expression over the --json envelope on stdin. Prints
# <parse-error> instead of aborting, so the script always reaches SUMMARY.
jq_py() { python3 -c "import sys,json;d=json.load(sys.stdin);$1" 2>/dev/null || echo "<parse-error>"; }

# ── Setup: a single CLI repo with a known file ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init local "$DVS_STORAGE"

mkfiles 2 1K data/raw   # data/raw/file_1.bin, data/raw/file_2.bin
dvs add --glob "data/**/*.bin"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: get --help matches spec block (L504-524) ==="
EXPECTED_HELP="$(cat <<'EOF'
Retrieves the given files from dvs storage. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided; to restore every tracked file, pass `--glob '**/*'`

Usage: dvs get [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Tracked files or directories to retrieve

Options:
  -g, --glob <GLOB>           Only retrieve the tracked files matching this glob, e.g. '*.csv'
  -r, --recursive             Recursively include files in subdirectories for directory inputs. Without this flag, directories return only their direct children
      --dry-run               Show what would be retrieved without making any actual changes
      --on-conflict <POLICY>  What to do with files whose local version dvs did not put there (local changes, or no record): abort changes nothing, skip leaves them and gets the rest, overwrite replaces them. Defaults to DVS_GET_ON_CONFLICT, then `[get] on_conflict` in dvs.toml, then abort [possible values: abort, skip, overwrite]
  -f, --force                 Replace files with local changes (same as --on-conflict overwrite)
  -h, --help                  Print help

Global Options:
      --json               Output results as JSON
      --emit-events        Stream operation results and transfer progress as NDJSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
EOF
)"
ACTUAL_HELP="$(dvs get --help)"
norm() { printf '%s\n' "$1" | sed 's/[[:space:]]*$//'; }
check "$(norm "$EXPECTED_HELP")" "$(norm "$ACTUAL_HELP")" "get --help matches spec block L504-524 (trailing-space normalized)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: the CLI requires a path or --glob (L505, L526) ==="
rc=0; dvs get >/dev/null 2>&1 || rc=$?
check "2" "$rc" "bare 'dvs get' (no path, no --glob) is a usage error, exit 2 (L216)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: action \`restored\` after deleting local copy (L461, L556) ==="
rm -f data/raw/file_1.bin
COPIED_JSON="$(rm -f data/raw/file_2.bin; dvs get --json data/raw/file_2.bin)"
COPIED_OUTCOME="$(printf '%s' "$COPIED_JSON" | jq_py 'r=d["files"][0];print(r["before"], r["action"])')"
check "absent restored" "$COPIED_OUTCOME" "before=absent, action=restored (JSON) for retrieved file_2.bin (L461)"
# Human-readable confirmation for file_1 (deleted above)
HR="$(dvs get data/raw/file_1.bin)"
HAS_FILE1="$(printf '%s' "$HR" | grep -c 'file_1.bin' || true)"
check "1" "$HAS_FILE1" "human-readable get lists restored file_1.bin (L531)"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "local file_1.bin restored on disk (L461)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: action \`unchanged\` when local already matches metadata (L462, L556) ==="
# file_1.bin is now present locally and matches metadata.
PRESENT_JSON="$(dvs get --json data/raw/file_1.bin)"
PRESENT_OUTCOME="$(printf '%s' "$PRESENT_JSON" | jq_py 'r=d["files"][0];print(r["before"], r["action"])')"
check "current unchanged" "$PRESENT_OUTCOME" "before=current, action=unchanged (JSON) for already-matching file_1.bin (L462)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: get resolves paths against METADATA folder not working tree (L42, L466) ==="
# Delete the local file entirely; metadata sidecar still exists -> get resolves it.
rm -f data/raw/file_1.bin
test ! -e data/raw/file_1.bin
RESOLVE_JSON="$(dvs get --json data/raw/file_1.bin)"
RESOLVE_OUTCOME="$(printf '%s' "$RESOLVE_JSON" | jq_py 'print(d["files"][0]["action"])')"
check "restored" "$RESOLVE_OUTCOME" "path with no working-tree file still resolved via metadata then restored (L42)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: -g / --glob short + long flags both work (L513) ==="
rm -f data/raw/file_1.bin data/raw/file_2.bin
SHORT_JSON="$(dvs get -g 'data/**/*.bin' --json)"
SHORT_N="$(printf '%s' "$SHORT_JSON" | jq_py 'print(len(d["files"]), d["counts"]["restored"])')"
check "2 2" "$SHORT_N" "-g glob retrieved 2 files (2 rows, counts.restored=2) (L513)"
rm -f data/raw/file_1.bin data/raw/file_2.bin
LONG_JSON="$(dvs get --glob 'data/**/*.bin' --json)"
LONG_N="$(printf '%s' "$LONG_JSON" | jq_py 'print(len(d["files"]), d["counts"]["restored"])')"
check "2 2" "$LONG_N" "--glob glob retrieved 2 files (2 rows, counts.restored=2) (L513)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --dry-run reports outcomes but makes NO changes (L44, L515) ==="
rm -f data/raw/file_1.bin
DRY_HR="$(dvs get --dry-run data/raw/file_1.bin)"
DRY_LISTS="$(printf '%s' "$DRY_HR" | grep -c 'file_1.bin' || true)"
check "1" "$DRY_LISTS" "--dry-run reports file_1.bin in output (L515)"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run wrote NO local file (L515)"
# JSON dry-run still shows the would-be action but writes nothing.
DRY_JSON="$(dvs get --dry-run --json data/raw/file_1.bin)"
DRY_OUTCOME="$(printf '%s' "$DRY_JSON" | jq_py 'print(d["dry_run"], d["files"][0]["action"], d["files"][0]["satisfied"], d["bytes"]["transferred"])')"
check "True would_restore True 0" "$DRY_OUTCOME" "--dry-run --json: dry_run=true, action=would_restore, satisfied=true, bytes.transferred=0 (L112, L554, L556)"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "--dry-run --json wrote NO local file (L515)"
# Restore for later use.
dvs get data/raw/file_1.bin >/dev/null

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --json output shape for get (L538-566) ==="
rm -f data/raw/file_1.bin
JSON_OUT="$(dvs get --json data/raw/file_1.bin)"
say "$JSON_OUT"
# One v1 envelope; rows with the spec's fixed keys (extra keys tolerated, L86).
JSON_OK="$(printf '%s' "$JSON_OUT" | python3 -c '
import json,sys
d=json.load(sys.stdin)
bad=[]
for k, v in (("v", 1), ("command", "get"), ("outcome", "complete"), ("dry_run", False), ("error", None), ("on_conflict", "abort"), ("source", "default")):
    if d.get(k, "<missing>") != v:
        bad.append("%s=%r" % (k, d.get(k, "<missing>")))
for k in ("counts", "bytes", "files", "started_at", "finished_at", "duration_ms"):
    if k not in d:
        bad.append("missing " + k)
c = d.get("counts", {})
ck = {"total", "succeeded", "restored", "replaced", "unchanged", "skipped", "not_run", "cancelled", "failed", "would_restore", "would_replace"}
if not ck <= set(c):
    bad.append("counts missing %s" % sorted(ck - set(c)))
b = d.get("bytes", {})
if not {"total", "to_transfer", "already_stored", "transferred"} <= set(b):
    bad.append("bytes=%r" % b)
rows = d.get("files") or []
if len(rows) != 1:
    bad.append("rows=%d" % len(rows))
else:
    o = rows[0]
    rk = {"path", "local_path", "before", "action", "satisfied", "size", "target_hash", "local_hash", "stored_copy", "code", "message", "hint"}
    if not rk <= set(o):
        bad.append("row missing %s" % sorted(rk - set(o)))
    if o.get("path") != "data/raw/file_1.bin":
        bad.append("path=%r" % o.get("path"))
    if o.get("action") != "restored" or o.get("satisfied") is not True:
        bad.append("action=%r satisfied=%r" % (o.get("action"), o.get("satisfied")))
    if not isinstance(o.get("size"), int):
        bad.append("size=%r" % o.get("size"))
    if o.get("code") is not None:
        bad.append("code=%r" % o.get("code"))
print("ok" if not bad else ";".join(bad))
' 2>&1 || echo "<parse-error>")"
check "ok" "$JSON_OK" "--json is a v1 get envelope with on_conflict/source/counts/bytes and spec row keys (L538-566)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: exit code 1 if one or more files could not be retrieved (L568-569) ==="
# (a) requested path has no metadata at all -> a not_tracked row -> exit 1.
rc=0; NT_JSON="$(dvs get --json does/not/exist.bin 2>/dev/null)" || rc=$?
check "1" "$rc" "exit 1 when requested path has no metadata (L568)"
NT_CODE="$(printf '%s' "$NT_JSON" | jq_py 'r=d["files"][0];print(r["before"], r["code"])')"
check "not_tracked not_tracked" "$NT_CODE" "untracked path is a row with before=not_tracked, code=not_tracked (L466, L558)"
# A path with no metadata mixed with a valid one refuses the WHOLE batch
# (#240, then on_conflict=abort): a known problem writes nothing, the valid
# row is held back as batch_aborted, and the command exits 1.
rm -f data/raw/file_1.bin
rc=0; MIX_JSON="$(dvs get --json data/raw/file_1.bin no/such.bin 2>/dev/null)" || rc=$?
check "1" "$rc" "untracked path mixed with a valid one refuses the whole batch, exit 1 (L472, L569)"
check "no" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "all-or-nothing: valid file NOT retrieved when a sibling path is untracked (L472)"
MIX_ROW="$(printf '%s' "$MIX_JSON" | jq_py 'r=[f for f in d["files"] if f["path"]=="data/raw/file_1.bin"][0];print(d["outcome"], r["action"], r["code"])')"
check "aborted not_run batch_aborted" "$MIX_ROW" "outcome=aborted, the valid row is action=not_run, code=batch_aborted (L473-474, L551)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: local changes are a conflict: abort writes nothing, exit 3; --force replaces (L464, L472, L517, L569) ==="
dvs get data/raw/file_1.bin >/dev/null
rm -f data/raw/file_2.bin
printf 'LOCAL EDIT' >> data/raw/file_1.bin
EDITED_HASH="$(shasum data/raw/file_1.bin | cut -d' ' -f1)"
rc=0; CF_JSON="$(dvs get --json data/raw/file_1.bin data/raw/file_2.bin 2>/dev/null)" || rc=$?
check "3" "$rc" "a file with local changes under on_conflict=abort exits 3 (L569)"
CF_ROW="$(printf '%s' "$CF_JSON" | jq_py 'r=[f for f in d["files"] if f["path"]=="data/raw/file_1.bin"][0];print(d["outcome"], r["before"], r["action"], r["code"])')"
check "aborted modified not_run local_changes" "$CF_ROW" "conflict row: before=modified, action=not_run, code=local_changes (L464, L556, L558)"
check "$EDITED_HASH" "$(shasum data/raw/file_1.bin | cut -d' ' -f1)" "abort kept the local edit unchanged (L472)"
check "no" "$([ -f data/raw/file_2.bin ] && echo yes || echo no)" "abort wrote nothing: absent file_2.bin NOT restored (L472)"
rc=0; FORCE_JSON="$(dvs get --json --force data/raw/file_1.bin 2>/dev/null)" || rc=$?
check "0" "$rc" "--force exits 0 (L517)"
FORCE_ROW="$(printf '%s' "$FORCE_JSON" | jq_py 'r=d["files"][0];print(r["action"], d["on_conflict"])')"
check "replaced overwrite" "$FORCE_ROW" "--force replaces the file: action=replaced, on_conflict=overwrite (L517, L556)"
dvs get data/raw/file_2.bin >/dev/null

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: downloaded bytes verified before install; a mismatch fails that file (L478-480, L559) ==="
# Force a GENUINE hash mismatch: replace the stored blob with a VALID zstd
# stream whose decompressed content differs from what the metadata hash
# expects. get will decompress fine, hash it, detect the mismatch, and fail
# that file without installing it.
mkfiles 1 1K data/tamper   # data/tamper/file_1.bin
dvs add data/tamper/file_1.bin >/dev/null
THASH="$(python3 -c 'import json;print(json.load(open(".dvs/data/tamper/file_1.bin.dvs"))["hashes"]["blake3"])')"
TBLOB="$DVS_STORAGE/${THASH:0:2}/${THASH:2}"
say "stored blob: $TBLOB"
# Build a valid zstd stream of different random bytes and overwrite the blob.
TMP_RAW="$(mktemp "${TMPDIR:-/tmp}/dvs_tamper_raw_XXXX")"
TMP_ZST="$(mktemp "${TMPDIR:-/tmp}/dvs_tamper_zst_XXXX")"
head -c 2048 /dev/urandom > "$TMP_RAW"
zstd -q -f "$TMP_RAW" -o "$TMP_ZST"
chmod +w "$TBLOB"
cp "$TMP_ZST" "$TBLOB"
rm -f "$TMP_RAW" "$TMP_ZST"
rm -f data/tamper/file_1.bin
rc=0; TAMPER_OUT="$(dvs get data/tamper/file_1.bin 2>&1)" || rc=$?
say "$TAMPER_OUT"
check "1" "$rc" "exit 1 on hash mismatch (L478, L568)"
check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "no file installed on hash mismatch (L478-480)"
# JSON form of the same failure: codes are the API, not the message (L85).
rm -f data/tamper/file_1.bin
rc=0; TAMPER_JSON="$(dvs get --json data/tamper/file_1.bin 2>/dev/null)" || rc=$?
TAMPER_ROW="$(printf '%s' "$TAMPER_JSON" | jq_py 'r=d["files"][0];print(r["action"], r["code"], d["counts"]["failed"])')"
check "failed hash_mismatch 1" "$TAMPER_ROW" "--json row action=failed, code=hash_mismatch, counts.failed=1 (L556, L559)"
check "no" "$([ -f data/tamper/file_1.bin ] && echo yes || echo no)" "no file installed on hash mismatch (--json path) (L478-480)"

# Genuine partial failure: one good file + the tampered file -> exit 1 (L569),
# good file still retrieved (failures are per file, L479-480).
rm -f data/raw/file_1.bin data/tamper/file_1.bin
rc=0; dvs get data/raw/file_1.bin data/tamper/file_1.bin >/dev/null 2>&1 || rc=$?
check "1" "$rc" "exit 1 on partial failure: 1 good + 1 hash-mismatch file (L569)"
check "yes" "$([ -f data/raw/file_1.bin ] && echo yes || echo no)" "good file still retrieved during partial failure (L479-480)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
