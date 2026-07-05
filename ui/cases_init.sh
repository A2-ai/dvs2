#!/usr/bin/env bash

# Exhaustive CLI scenario coverage for `dvs init` (specs.md "init" section).
# Per scenario: prints expected vs actual and a PASS/FAIL verdict, then a
# summary. Scripts assert the SPEC-CORRECT behavior. Surprising-but-spec-allowed
# behavior is documented inline via NOTE headers.
# CLI only. The `dvs` binary is assumed already on PATH (do not reinstall).

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

# Track every dir we create so we can self-clean ONLY our own dirs at the end
# (concurrency rule: never run cleanup.sh, never touch other agents' dirs).
CREATED=()
track() { CREATED+=("$@"); }

# Make a fresh empty repo dir + a sibling (outside-repo) storage dir.
#   mkpair <name>   ->   sets REPO and STORE globals (both created)
# Storage lives directly under ui/ so it is NEVER inside the repo.
mkpair() {
  local name="$1"
  REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_${name}"
  STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_${name}"
  mkdir -p "$REPO" "$STORE"
  track "$REPO" "$STORE"
}

# Make only a fresh empty repo dir (caller decides storage).
mkrepo() {
  local name="$1"
  REPO="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_${name}"
  mkdir -p "$REPO"
  track "$REPO"
}

# ── Setup: base temp repo + sibling storage ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"
track "$DVS_REPO" "$DVS_STORAGE"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init in an empty dir (happy path; dvs.toml + .dvs created) ==="
cd "$DVS_REPO"
rc=0
dvs init "$DVS_STORAGE" || rc=$?
check "0" "$rc" "init in empty dir exits 0"
toml_exists=no; [ -f "$DVS_REPO/dvs.toml" ] && toml_exists=yes
check "yes" "$toml_exists" "init creates dvs.toml in cwd"
meta_exists=no; [ -d "$DVS_REPO/.dvs" ] && meta_exists=yes
check "yes" "$meta_exists" "init creates default .dvs metadata folder"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: dvs.toml ALREADY exists in target dir (error, exit 1, toml untouched) ==="
# Snapshot the existing toml, re-init, confirm it errors and leaves toml byte-identical.
toml_before="$(cat "$DVS_REPO/dvs.toml")"
rc=0
dvs init "$DVS_STORAGE" || rc=$?
toml_after="$(cat "$DVS_REPO/dvs.toml")"
check "1" "$rc" "re-init in dir with existing dvs.toml exits 1"
same_toml=no; [ "$toml_before" = "$toml_after" ] && same_toml=yes
check "yes" "$same_toml" "existing dvs.toml left untouched after rejected re-init"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init in a SUBDIR of an existing project (succeeds — local check, L81) ==="
mkpair sub
mkdir -p "$DVS_REPO/nested"
cd "$DVS_REPO/nested"
rc=0
dvs init "$STORE" || rc=$?
nested_toml=no; [ -f "$DVS_REPO/nested/dvs.toml" ] && nested_toml=yes
check "0" "$rc" "nested init exits 0 despite parent dvs.toml (local check)"
check "yes" "$nested_toml" "nested dvs.toml created"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init with RELATIVE storage path ==="
# NOTE: spec is silent on how the path is recorded. Observed: the relative path
# is stored VERBATIM (not canonicalized) in dvs.toml backend.path.
mkrepo rel
cd "$REPO"
REL_STORE_ABS="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_rel"
track "$REL_STORE_ABS"
rc=0
dvs init ../"dvs_storage_cli_${RUN_SUFFIX}_rel" || rc=$?
check "0" "$rc" "init with relative storage path exits 0"
store_made=no; [ -d "$REL_STORE_ABS" ] && store_made=yes
check "yes" "$store_made" "relative storage dir created"
rel_path="$(grep -E '^path' "$REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
say "--- recorded backend.path = [$rel_path] (relative, verbatim) ---"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init with ABSOLUTE storage path ==="
mkpair abs
cd "$REPO"
rc=0
dvs init "$STORE" || rc=$?
check "0" "$rc" "init with absolute storage path exits 0"
abs_path="$(grep -E '^path' "$REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
check "$STORE" "$abs_path" "absolute storage path recorded verbatim in dvs.toml"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: storage path INSIDE the repo (rejected: 'within the repository', L83) ==="
mkrepo inside
cd "$REPO"
rc=0
err_out="$(dvs init ./inside_storage 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
check "1" "$rc" "init with storage inside repo exits 1"
mentions_within=no
printf '%s' "$err_out" | grep -qi "within the repository" && mentions_within=yes
check "yes" "$mentions_within" "error message mentions 'within the repository'"
no_toml=yes; [ -f "$REPO/dvs.toml" ] && no_toml=no
check "yes" "$no_toml" "no dvs.toml created when storage rejected"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: storage path already exists and is EMPTY (ok) ==="
mkpair emptystore
# STORE already exists (mkdir -p) and is empty.
cd "$REPO"
rc=0
dvs init "$STORE" || rc=$?
check "0" "$rc" "init into pre-existing EMPTY storage dir exits 0"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: storage path already exists and is NON-EMPTY (document) ==="
# NOTE: spec gives no guard against a non-empty storage dir. Observed: init
# SUCCEEDS and reuses the directory (no pre-existing-content check).
mkpair fullstore
echo "stale" > "$STORE/preexisting.txt"
cd "$REPO"
rc=0
dvs init "$STORE" || rc=$?
check "0" "$rc" "init into pre-existing NON-EMPTY storage dir exits 0 (no guard; documented)"
junk_kept=no; [ -f "$STORE/preexisting.txt" ] && junk_kept=yes
check "yes" "$junk_kept" "pre-existing file in storage left intact"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --root-dir <EXISTING dir> (toml lands there, not cwd) (L105) ==="
ROOTDIR="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_rootexist"
RD_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_rootexist"
CWD_PROBE="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_cwdprobe"
mkdir -p "$ROOTDIR" "$RD_STORE" "$CWD_PROBE"
track "$ROOTDIR" "$RD_STORE" "$CWD_PROBE"
cd "$CWD_PROBE"
rc=0
dvs init --root-dir "$ROOTDIR" "$RD_STORE" || rc=$?
check "0" "$rc" "init --root-dir <existing> exits 0"
rootdir_toml=no; [ -f "$ROOTDIR/dvs.toml" ] && rootdir_toml=yes
cwd_toml=present; [ -f "$CWD_PROBE/dvs.toml" ] || cwd_toml=absent
check "yes" "$rootdir_toml" "--root-dir places dvs.toml in given root"
check "absent" "$cwd_toml" "--root-dir does NOT create dvs.toml in cwd"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --root-dir <NONEXISTENT dir> (error or created? document) ==="
# NOTE: Observed: init ERRORS (canonicalize of the missing root-dir fails). The
# directory is NOT created. spec is silent; documenting CLI behavior = reject.
NOROOT="$SCRIPT_DIR/dvs_repo_cli_${RUN_SUFFIX}_noroot"
NOROOT_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_noroot"
mkdir -p "$NOROOT_STORE"
track "$NOROOT" "$NOROOT_STORE"
rm -rf "$NOROOT"   # ensure it does NOT exist
cd "$SCRIPT_DIR"
rc=0
err_out="$(dvs init --root-dir "$NOROOT" "$NOROOT_STORE" 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
check "1" "$rc" "init --root-dir <nonexistent> exits 1 (rejected; documented)"
root_created=no; [ -d "$NOROOT" ] && root_created=yes
check "no" "$root_created" "nonexistent --root-dir is NOT auto-created"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --metadata-folder-name custom (.mymeta created, .dvs not) (L107) ==="
mkpair mfn
cd "$REPO"
rc=0
dvs init --metadata-folder-name .mymeta "$STORE" || rc=$?
check "0" "$rc" "init --metadata-folder-name custom exits 0"
custom_meta=no; [ -d "$REPO/.mymeta" ] && custom_meta=yes
default_meta=absent; [ -d "$REPO/.dvs" ] && default_meta=present
check "yes" "$custom_meta" "custom .mymeta folder created"
check "absent" "$default_meta" "default .dvs folder NOT created when custom name given"
toml_has_mfn=no
grep -q 'metadata_folder_name = ".mymeta"' "$REPO/dvs.toml" && toml_has_mfn=yes
check "yes" "$toml_has_mfn" "dvs.toml records metadata_folder_name = .mymeta"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --metadata-folder-name COLLIDING with an existing dir (document) ==="
# NOTE: Observed: init ERRORS ("File exists") when the metadata folder name
# already exists as a directory. It does NOT reuse the existing folder. The
# pre-existing folder is left intact (cleanup does not remove a folder that
# existed beforehand, per L87).
mkpair mfncollide
mkdir -p "$REPO/.existingmeta"
echo "keepme" > "$REPO/.existingmeta/preexisting.txt"
cd "$REPO"
rc=0
err_out="$(dvs init --metadata-folder-name .existingmeta "$STORE" 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
check "1" "$rc" "init --metadata-folder-name colliding-with-existing-dir exits 1 (documented)"
pre_kept=no; [ -f "$REPO/.existingmeta/preexisting.txt" ] && pre_kept=yes
check "yes" "$pre_kept" "pre-existing colliding metadata folder content left intact"
no_toml=yes; [ -f "$REPO/dvs.toml" ] && no_toml=no
check "yes" "$no_toml" "dvs.toml cleaned up / not left behind after collision failure"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --no-compression (toml records 'none') (L111) ==="
mkpair nc
cd "$REPO"
rc=0
dvs init --no-compression "$STORE" || rc=$?
check "0" "$rc" "init --no-compression exits 0"
toml_comp="$(grep -E '^compression' "$REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
check "none" "$toml_comp" "--no-compression records compression=none in dvs.toml"
# Sanity: default (no flag) records zstd.
mkpair zstd
cd "$REPO"
dvs init "$STORE"
def_comp="$(grep -E '^compression' "$REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
check "zstd" "$def_comp" "default init records compression=zstd in dvs.toml"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --group <user's OWN group> (applied + recorded) (L109) ==="
USER_GROUP="$(id -gn)"
mkpair grp
cd "$REPO"
rc=0
dvs init --group "$USER_GROUP" "$STORE" || rc=$?
check "0" "$rc" "init --group <own group> exits 0"
store_grp="$(stat -f %Sg "$STORE" 2>/dev/null || stat -c %G "$STORE")"
toml_grp="$(grep -E '^group' "$REPO/dvs.toml" | tr -d ' "' | cut -d= -f2 || true)"
say "--- toml group=[$toml_grp] storage dir group=[$store_grp] user group=[$USER_GROUP] ---"
check "$USER_GROUP" "$store_grp" "--group sets unix group on storage dir"
check "$USER_GROUP" "$toml_grp" "--group recorded in dvs.toml backend.group"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --group <NONEXISTENT group> (error, document) (L109) ==="
# NOTE: Observed: init ERRORS with "Group '<name>' not found". Documenting.
mkpair grpbad
cd "$REPO"
rc=0
err_out="$(dvs init --group zzznosuchgrp_dvs "$STORE" 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
check "1" "$rc" "init --group <nonexistent> exits 1 (documented)"
mentions_grp=no
printf '%s' "$err_out" | grep -qi "not found" && mentions_grp=yes
check "yes" "$mentions_grp" "error message mentions group not found"
no_toml=yes; [ -f "$REPO/dvs.toml" ] && no_toml=no
check "yes" "$no_toml" "no dvs.toml left behind after bad-group failure"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --threads 0 (auto) and --threads N ==="
mkpair t0
cd "$REPO"
rc=0
dvs init --threads 0 "$STORE" || rc=$?
check "0" "$rc" "init --threads 0 (auto-detect) exits 0"
mkpair tN
cd "$REPO"
rc=0
dvs init --threads 4 "$STORE" || rc=$?
check "0" "$rc" "init --threads 4 exits 0"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --json (valid JSON, status initialized) (L101) ==="
mkpair json
cd "$REPO"
json_out="$(dvs init --json "$STORE")"
say "--- json output: $json_out ---"
json_status="$(printf '%s' "$json_out" | python3 -c '
import json,sys
try:
    e = json.load(sys.stdin)
except Exception:
    print("noparse"); sys.exit()
print(e.get("status",""))
')"
check "initialized" "$json_status" "init --json emits {\"status\":\"initialized\"}"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: PATH argument missing (clap usage error, exit 2) (L98) ==="
mkrepo nopath
cd "$REPO"
rc=0
err_out="$(dvs init 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
check "2" "$rc" "init with no PATH arg exits 2 (clap usage error)"
mentions_required=no
printf '%s' "$err_out" | grep -qi "required" && mentions_required=yes
check "yes" "$mentions_required" "clap usage error mentions required <PATH>"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: --help text matches spec block (L92-115) ==="
# Extract the fenced ```shell block under '### init' that begins with the
# `❯ dvs init --help` line, dropping that prompt line itself.
EXPECTED_HELP="$(awk '
  /^### init$/      { in_init=1; next }
  in_init && /^### / { exit }
  in_init && /^```shell$/ { in_block=1; next }
  in_init && in_block && /^```$/ { exit }
  in_init && in_block {
    if ($0 ~ /dvs init --help/) next
    print
  }
' "$SCRIPT_DIR/../specs.md")"
ACTUAL_HELP="$(dvs init --help)"
say "--- diff (expected vs actual) ---"
help_diff="$(diff <(printf '%s\n' "$EXPECTED_HELP") <(printf '%s\n' "$ACTUAL_HELP") || true)"
say "${help_diff:-<no diff>}"
help_match=no; [ -z "$help_diff" ] && help_match=yes
check "yes" "$help_match" "init --help matches spec block exactly (order + text)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: partial-failure cleanup + retry (L86-87) ==="
# Force storage creation to fail AFTER local artifacts: point storage under a
# non-directory (/dev/null) so mkdir fails. Expect dvs.toml + .dvs removed.
mkrepo pf
cd "$REPO"
rc=0
err_out="$(dvs init /dev/null/cannot_create_here 2>&1)" || rc=$?
say "--- stderr: $err_out ---"
pf_toml=absent; [ -f "$REPO/dvs.toml" ] && pf_toml=present
pf_meta=absent; [ -d "$REPO/.dvs" ] && pf_meta=present
say "--- after failed init: dvs.toml=$pf_toml .dvs=$pf_meta ---"
check "1" "$rc" "init with unwritable storage exits non-zero"
check "absent" "$pf_toml" "dvs.toml cleaned up after partial failure"
check "absent" "$pf_meta" "metadata folder cleaned up after partial failure"
# Retry into a valid storage proves the dir is left in a clean re-initable state.
PF_STORE="$SCRIPT_DIR/dvs_storage_cli_${RUN_SUFFIX}_pf"
mkdir -p "$PF_STORE"
track "$PF_STORE"
rc=0
dvs init "$PF_STORE" || rc=$?
check "0" "$rc" "retry init succeeds after partial-failure cleanup"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init writes audit entry (action=init, settings, project_path) (L377-395) ==="
AUDIT="$DVS_STORAGE/audit.log.jsonl"
audit_exists=no; [ -f "$AUDIT" ] && audit_exists=yes
check "yes" "$audit_exists" "audit.log.jsonl exists in storage after init"
init_line="$(grep -m1 '"init"' "$AUDIT" || true)"
say "--- audit.log.jsonl (first init entry): $init_line ---"
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
      and "operation_id" in e and "timestamp" in e and "user" in e)
print("yes" if ok else "no")
')"
check "yes" "$audit_ok" "init audit entry has action.init{settings,project_path} (+ op_id/ts/user)"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== Scenario: init THEN immediately add works (end-to-end) (L77) ==="
mkpair e2e
cd "$REPO"
git init -q
rc=0
dvs init "$STORE" || rc=$?
check "0" "$rc" "init (e2e) exits 0"
mkrandfile "$REPO/data/payload.bin" 1K
rc=0
add_out="$(dvs add data/payload.bin 2>&1)" || rc=$?
say "--- add output: $add_out ---"
check "0" "$rc" "add immediately after init exits 0"
sidecar=no; [ -f "$REPO/.dvs/data/payload.bin.dvs" ] && sidecar=yes
check "yes" "$sidecar" "add after init writes the expected .dvs sidecar"

# ─────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="

# ── Self-clean ONLY our own dirs (never cleanup.sh, never others' dirs) ──
{ set +x; } 2>/dev/null
cd "$SCRIPT_DIR"
for d in "${CREATED[@]}"; do
  rm -rf "$d"
done
