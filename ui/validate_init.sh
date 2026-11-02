#!/usr/bin/env bash

# Validate CLI spec claims for `dvs init` (specs.md).
# Per claim: prints expected vs actual and a PASS/FAIL verdict, then a summary.
# CLI only. The `dvs` binary is assumed already on PATH (do not reinstall).
# Claims come from specs.md, init, unless a claim names another section.

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
  set -x
}

# ── Setup: temp repo + sibling storage ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: init local creates dvs.toml in current folder ==="
cd "$DVS_REPO"
dvs init local "$DVS_STORAGE"
toml_exists=no; [ -f "$DVS_REPO/dvs.toml" ] && toml_exists=yes
check "yes" "$toml_exists" "init local creates dvs.toml in cwd"
meta_exists=no; [ -d "$DVS_REPO/.dvs" ] && meta_exists=yes
check "yes" "$meta_exists" "init local creates default .dvs metadata folder"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: the pre-backend form 'dvs init <PATH>' exits 2 and suggests 'dvs init local <PATH>' ==="
OLD_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_old"
DVS_STORAGE_OLD="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_old"
mkdir -p "$OLD_REPO" "$DVS_STORAGE_OLD"
cd "$OLD_REPO"
rc=0
old_out="$(dvs init "$DVS_STORAGE_OLD" 2>&1)" || rc=$?
say "$old_out"
check "2" "$rc" "'dvs init <PATH>' exits 2"
suggests=no
printf '%s' "$old_out" | grep -qF "dvs init local $DVS_STORAGE_OLD" && suggests=yes
check "yes" "$suggests" "'dvs init <PATH>' suggests 'dvs init local <PATH>'"
old_toml=absent; [ -f "$OLD_REPO/dvs.toml" ] && old_toml=present
check "absent" "$old_toml" "'dvs init <PATH>' writes no dvs.toml"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: init errors if dvs.toml already exists in target dir ==="
cd "$DVS_REPO"
rc=0
dvs init local "$DVS_STORAGE" || rc=$?
check "1" "$rc" "re-init in dir with existing dvs.toml exits 1"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: check is LOCAL, parent dvs.toml does NOT block nested init ==="
DVS_STORAGE_NESTED="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_nested"
mkdir -p "$DVS_REPO/nested" "$DVS_STORAGE_NESTED"
cd "$DVS_REPO/nested"
rc=0
dvs init local "$DVS_STORAGE_NESTED" || rc=$?
nested_toml=no; [ -f "$DVS_REPO/nested/dvs.toml" ] && nested_toml=yes
check "0" "$rc" "nested init exit code 0 despite parent dvs.toml"
check "yes" "$nested_toml" "nested dvs.toml created"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: init rejects a storage path inside the repository root ==="
IN_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_inside"
mkdir -p "$IN_REPO"
cd "$IN_REPO"
rc=0
dvs init local "$IN_REPO/store" || rc=$?
in_toml=absent; [ -f "$IN_REPO/dvs.toml" ] && in_toml=present
check "1" "$([ "$rc" -ne 0 ] && echo 1 || echo 0)" "storage inside the repo root exits non-zero"
check "absent" "$in_toml" "storage inside the repo root writes no dvs.toml"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --root-dir <DIR> creates dvs.toml in that root instead of cwd ==="
ROOTDIR="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_root"
DVS_STORAGE_ROOT="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_root"
mkdir -p "$ROOTDIR" "$DVS_STORAGE_ROOT"
# Call from a different cwd to prove dvs.toml lands in --root-dir, not cwd.
CWD_PROBE="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_cwdprobe"
mkdir -p "$CWD_PROBE"
cd "$CWD_PROBE"
dvs init local --root-dir "$ROOTDIR" "$DVS_STORAGE_ROOT"
rootdir_toml=no; [ -f "$ROOTDIR/dvs.toml" ] && rootdir_toml=yes
cwd_toml=present; [ -f "$CWD_PROBE/dvs.toml" ] || cwd_toml=absent
check "yes" "$rootdir_toml" "--root-dir places dvs.toml in given root"
check "absent" "$cwd_toml" "--root-dir does NOT create dvs.toml in cwd"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --metadata-folder-name <NAME> uses custom metadata folder ==="
MFN_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_mfn"
DVS_STORAGE_MFN="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_mfn"
mkdir -p "$MFN_REPO" "$DVS_STORAGE_MFN"
cd "$MFN_REPO"
dvs init local --metadata-folder-name .mymeta "$DVS_STORAGE_MFN"
custom_meta=no; [ -d "$MFN_REPO/.mymeta" ] && custom_meta=yes
default_meta=absent; [ -d "$MFN_REPO/.dvs" ] && default_meta=present
toml_has_mfn=no
grep -q 'metadata_folder_name = ".mymeta"' "$MFN_REPO/dvs.toml" && toml_has_mfn=yes
check "yes" "$custom_meta" "--metadata-folder-name creates custom .mymeta folder"
check "absent" "$default_meta" "default .dvs folder NOT created when custom name given"
check "yes" "$toml_has_mfn" "dvs.toml records metadata_folder_name = .mymeta"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --no-compression -> compression \"none\" (verify via add metadata, specs.md, Compression) ==="
NC_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_nc"
DVS_STORAGE_NC="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_nc"
mkdir -p "$NC_REPO" "$DVS_STORAGE_NC"
cd "$NC_REPO"
dvs init local --no-compression "$DVS_STORAGE_NC"
toml_comp=""
toml_comp="$(grep -E '^compression' "$NC_REPO/dvs.toml" | tr -d ' "' | cut -d= -f2)"
check "none" "$toml_comp" "--no-compression records compression=none in dvs.toml"
# Verify it propagates to per-file add metadata (compression field).
git init -q
mkfiles 1 1K data
dvs add data/file_1.bin
NC_META="$(find "$NC_REPO/.dvs" -type f -name '*.dvs' | head -n1)"
say "--- no-compression .dvs sidecar ---"
cat "$NC_META"
meta_comp="$(grep -oE '"compression": *"[^"]*"' "$NC_META" | head -n1 | cut -d'"' -f4 || true)"
check "none" "$meta_comp" "add metadata records compression=none under --no-compression"
# Sanity: default (zstd) repo records zstd in add metadata.
say "--- default (zstd) repo add sidecar for contrast ---"
cd "$DVS_REPO"
git init -q
mkfiles 1 1K data
dvs add data/file_1.bin
DEF_META="$(find "$DVS_REPO/.dvs" -type f -name '*.dvs' | head -n1)"
cat "$DEF_META"
def_comp="$(grep -oE '"compression": *"[^"]*"' "$DEF_META" | head -n1 | cut -d'"' -f4 || true)"
check "zstd" "$def_comp" "default add metadata records compression=zstd"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --group <GRP> sets unix group on storage dir/files ==="
USER_GROUP="$(id -gn)"
GRP_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_grp"
DVS_STORAGE_GRP="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_grp"
mkdir -p "$GRP_REPO" "$DVS_STORAGE_GRP"
cd "$GRP_REPO"
dvs init local --group "$USER_GROUP" "$DVS_STORAGE_GRP"
# stat group owner of the storage dir (BSD stat on macOS: -f %Sg).
store_grp="$(stat -f %Sg "$DVS_STORAGE_GRP" 2>/dev/null || stat -c %G "$DVS_STORAGE_GRP")"
toml_grp="$(grep -E '^group' "$GRP_REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
say "--- group toml line and storage dir group ---"
say "toml group=[$toml_grp] storage dir group=[$store_grp] user group=[$USER_GROUP]"
check "$USER_GROUP" "$store_grp" "--group sets unix group on storage dir"
check "$USER_GROUP" "$toml_grp" "--group recorded in dvs.toml backend.group"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: --json prints the v1 envelope with project_root (specs.md, Machine output) ==="
JSON_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_json"
DVS_STORAGE_JSON="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_json"
mkdir -p "$JSON_REPO" "$DVS_STORAGE_JSON"
cd "$JSON_REPO"
json_out="$(dvs init local --json "$DVS_STORAGE_JSON")"
say "--- json output ---"
say "$json_out"
json_ok="$(printf '%s' "$json_out" | python3 -c '
import json, os, sys
try:
    d = json.load(sys.stdin)
except Exception as e:
    print("noparse: %s" % e); sys.exit()
bad = []
for k, v in (("v", 1), ("command", "init"), ("outcome", "complete"), ("dry_run", False), ("error", None)):
    if d.get(k, "<missing>") != v:
        bad.append("%s=%r" % (k, d.get(k, "<missing>")))
for k in ("started_at", "finished_at", "duration_ms", "project_root"):
    if k not in d:
        bad.append("missing " + k)
root = d.get("project_root") or ""
if not os.path.isabs(root) or os.path.realpath(root) != os.path.realpath(sys.argv[1]):
    bad.append("project_root=%r" % root)
print("ok" if not bad else ";".join(bad))
' "$JSON_REPO" 2>&1 || echo "<parse-error>")"
check "ok" "$json_ok" "init --json is a v1 envelope (command=init, outcome=complete, project_root = repo root)"
# The JSON error envelope for a failing init (re-init).
rc=0
json_err="$(dvs --json init local "$DVS_STORAGE_JSON")" || rc=$?
say "$json_err"
err_ok="$(printf '%s' "$json_err" | python3 -c '
import json, sys
d = json.load(sys.stdin)
e = d.get("error") or {}
ok = d.get("v") == 1 and d.get("command") == "init" and d.get("outcome") == "failed" and isinstance(e.get("code"), str) and "message" in e
print("ok" if ok else "bad: %r" % d)
' 2>&1 || echo "<parse-error>")"
check "ok" "$err_ok" "failing init --json is an error envelope (outcome=failed, error.code set)"
check "1" "$rc" "failing init --json exits 1"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: partial-failure best-effort cleanup ==="
# Storage creation must fail AFTER dvs.toml is conceptually written; force by
# pointing storage at a path under a non-directory so mkdir fails.
PF_REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_pf"
mkdir -p "$PF_REPO"
cd "$PF_REPO"
rc=0
dvs init local /dev/null/cannot_create_here || rc=$?
pf_toml=absent; [ -f "$PF_REPO/dvs.toml" ] && pf_toml=present
pf_meta=absent; [ -d "$PF_REPO/.dvs" ] && pf_meta=present
say "--- after failed init: dvs.toml=$pf_toml .dvs=$pf_meta ---"
check "1" "$rc" "init with unwritable storage exits non-zero"
check "absent" "$pf_toml" "dvs.toml cleaned up after partial failure"
check "absent" "$pf_meta" "metadata folder cleaned up after partial failure"
# Prove retry is possible after cleanup.
DVS_STORAGE_PF="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_pf"
mkdir -p "$DVS_STORAGE_PF"
rc=0
dvs init local "$DVS_STORAGE_PF" || rc=$?
check "0" "$rc" "retry init succeeds after partial-failure cleanup"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Claim: init writes audit entry action=init with settings + project_path (specs.md, Audit trail) ==="
AUDIT="$DVS_STORAGE/audit.log.jsonl"
audit_exists=no; [ -f "$AUDIT" ] && audit_exists=yes
check "yes" "$audit_exists" "audit.log.jsonl exists in storage after init"
say "--- audit.log.jsonl (first init entry) ---"
init_line="$(grep -m1 '"init"' "$AUDIT" || true)"
say "$init_line"
# Use python to assert structure.
audit_ok="$(printf '%s' "$init_line" | python3 -c '
import json,sys
try:
    e = json.load(sys.stdin)
except Exception:
    print("noparse"); sys.exit()
act = e.get("action", {})
ok = ("init" in act
      and "settings" in act["init"]
      and "project_path" in act["init"]
      and "operation_id" in e and "user" in e
      and isinstance(e.get("timestamp"), int))
print("yes" if ok else "no")
')"
check "yes" "$audit_ok" "init audit entry has action.init with settings + project_path (+ op_id, user, unix-seconds timestamp)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
