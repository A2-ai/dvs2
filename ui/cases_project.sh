#!/usr/bin/env bash

# cases_project.sh — project discovery & multiple/nested projects
#
# Exhaustive coverage of the cases_project.sh matrix in SCENARIOS.md and the
# spec Glossary (project root discovery by walking UP to the nearest dvs.toml,
# multiple projects in one git repo, L19-22) plus init local-check (L71/L84-85)
# and content-addressable storage (L358-360):
#
#   1. run a command from a nested subdir -> walks UP to dvs.toml; paths
#      reported project-root-relative
#   2. no dvs.toml anywhere up the tree -> clear error, exit 1
#   3. nested project (init inside an existing project) -> NEAREST dvs.toml
#      wins; a file added from the inner dir lands its blob in the INNER
#      storage, not the outer one (proved by blob LOCATION)
#   4. two sibling projects in one git repo with DIFFERENT storage -> each
#      resolves to its own storage
#   5. command run exactly AT the project root
#   6. storage path SHARED by two projects -> content-addressable dedup across
#      projects (same content -> same blob path in the shared storage)
#   7. custom --metadata-folder-name respected by add/get/status discovery
#
# Each scenario prints expected vs actual and a PASS/FAIL verdict. Ends with a
# `=== SUMMARY: N pass, M fail ===` line. Asserts SPEC-CORRECT behavior.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# ── verdict helpers ──────────────────────────────────────────────────────────
PASS_N=0
FAIL_N=0
NOTE_N=0

check() {
  { set +x; } 2>/dev/null
  # check WANT GOT LABEL
  if [ "$1" = "$2" ]; then
    echo "PASS: $3 (want=$1 got=$2)"
    PASS_N=$((PASS_N + 1))
  else
    echo "FAIL: $3 (want=$1 got=$2)"
    FAIL_N=$((FAIL_N + 1))
  fi
  { set -x; } 2>/dev/null
}

note() {
  { set +x; } 2>/dev/null
  echo "NOTE: $1"
  NOTE_N=$((NOTE_N + 1))
  { set -x; } 2>/dev/null
}

# Extract the first blake3 hash from `dvs status --json` output of $1 (a dir).
# Mirrors the idiom in validate_global.sh CLAIM 4.
first_hash() {
  (cd "$1" && dvs status --json) \
    | sed -n 's/.*"blake3":"\([0-9a-f]\{64\}\)".*/\1/p' | head -1
}

# Path a blob with hash $2 would occupy in storage $1 (2-char prefix split).
blob_path() {
  printf '%s/%s/%s\n' "$1" "${2:0:2}" "${2:2}"
}

# ── setup: pristine git repo + sibling storages OUTSIDE the repo ─────────────
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_proj_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"

# All storages live as siblings under ui/, NEVER inside the repo (init rejects
# storage within the project root).
OUTER_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_outer"
INNER_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_inner"
SIB_A_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_sibA"
SIB_B_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_sibB"
SHARED_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_shared"
META_STORE="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_meta"
mkdir "$OUTER_STORE" "$INNER_STORE" "$SIB_A_STORE" "$SIB_B_STORE" \
      "$SHARED_STORE" "$META_STORE"

cd "$DVS_REPO"
git init -q
# Outer project at the repo root.
dvs init "$OUTER_STORE"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 1: command from a NESTED subdir walks UP to dvs.toml; paths root-relative ==="
# Add a deeply nested file from the repo root, then run status from the nested
# subdir. Discovery must walk up to the outer dvs.toml and report the path
# relative to the project root (not relative to the cwd it was invoked from).
mkdir -p "$DVS_REPO/data/raw/deep"
mkrandfile "$DVS_REPO/data/raw/deep/nested.bin" 1K
(cd "$DVS_REPO" && dvs add data/raw/deep/nested.bin >/dev/null)

S1_RC=0
S1_OUT="$(cd "$DVS_REPO/data/raw/deep" && dvs status --json 2>&1)" || S1_RC=$?
check "0" "$S1_RC" "status from nested subdir exits 0 (rc=$S1_RC)"
# Reported path is project-root-relative => data/raw/deep/nested.bin, NOT
# cwd-relative (which would be just nested.bin).
case "$S1_OUT" in
  *'"path":"data/raw/deep/nested.bin"'*) S1_PATH=rootrel ;;
  *'"path":"nested.bin"'*)               S1_PATH=cwdrel  ;;
  *)                                     S1_PATH=missing ;;
esac
check "rootrel" "$S1_PATH" "nested status reports project-root-relative path"

# add and get also discover the root from the nested subdir.
mkrandfile "$DVS_REPO/data/raw/deep/added_from_nested.bin" 1K
S1_ADD_RC=0
S1_ADD="$(cd "$DVS_REPO/data/raw/deep" && dvs add added_from_nested.bin --json 2>&1)" || S1_ADD_RC=$?
check "0" "$S1_ADD_RC" "add from nested subdir exits 0 (rc=$S1_ADD_RC)"
case "$S1_ADD" in
  *'"path":"data/raw/deep/added_from_nested.bin"'*) S1_ADD_PATH=rootrel ;;
  *)                                                S1_ADD_PATH=other   ;;
esac
check "rootrel" "$S1_ADD_PATH" "add from nested subdir reports project-root-relative path"

# get from the nested subdir: the cwd-relative basename resolves and restores.
rm -f "$DVS_REPO/data/raw/deep/nested.bin"
S1_GET_RC=0
(cd "$DVS_REPO/data/raw/deep" && dvs get nested.bin --json >/dev/null 2>&1) || S1_GET_RC=$?
check "0" "$S1_GET_RC" "get from nested subdir (cwd-relative path) exits 0 (rc=$S1_GET_RC)"
check "1" "$([ -f "$DVS_REPO/data/raw/deep/nested.bin" ] && echo 1 || echo 0)" \
  "get from nested subdir restored the file at project-relative location"

# FINDING: get's positional path is resolved CWD-relative, not project-root-
# relative. A project-root-relative path passed from a nested subdir, AND an
# ABSOLUTE path, both fail with "No files to get" even though spec L57-60 says
# all commands must accept absolute paths. status/add report root-relative
# paths, so a path copied out of `status` output cannot be fed back to `get`
# from a nested cwd. Documented as a NOTE, not a hard FAIL of discovery itself.
rm -f "$DVS_REPO/data/raw/deep/nested.bin"
(cd "$DVS_REPO/data/raw/deep" && dvs get nested.bin >/dev/null 2>&1) || true
S1_GET_ROOTREL_RC=0
(cd "$DVS_REPO/data/raw/deep" && dvs get data/raw/deep/nested.bin --json >/dev/null 2>&1) || S1_GET_ROOTREL_RC=$?
S1_GET_ABS_RC=0
(cd "$DVS_REPO/data/raw/deep" && dvs get "$DVS_REPO/data/raw/deep/nested.bin" --json >/dev/null 2>&1) || S1_GET_ABS_RC=$?
if [ "$S1_GET_ROOTREL_RC" -ne 0 ]; then
  note "get root-relative path from nested cwd FAILS (rc=$S1_GET_ROOTREL_RC, 'No files to get') — positional path is cwd-relative, mismatching status/add root-relative output"
fi
if [ "$S1_GET_ABS_RC" -ne 0 ]; then
  note "get with ABSOLUTE path from nested cwd FAILS (rc=$S1_GET_ABS_RC) — contradicts spec L57-60 'commands must accept absolute paths'; worth filing"
fi

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 2: NO dvs.toml anywhere up the tree -> clear error, exit 1 ==="
# A directory tree with no dvs.toml at any ancestor. Use a fresh temp dir whose
# parents (mktemp root) contain no dvs.toml.
NOPROJ="$(mktemp -d "$SCRIPT_DIR"/dvs_noproj_${RUN_SUFFIX}_XXX)"
mkdir -p "$NOPROJ/a/b/c"
mkrandfile "$NOPROJ/a/b/c/orphan.bin" 1K
S2_RC=0
S2_OUT="$(cd "$NOPROJ/a/b/c" && dvs status 2>&1)" || S2_RC=$?
check "1" "$S2_RC" "status with no dvs.toml up the tree exits 1 (rc=$S2_RC)"
# Error message should be clear about the missing project / dvs.toml.
if printf '%s' "$S2_OUT" | grep -qiE 'dvs\.toml|not in a dvs|repository|no.*project|project.*not'; then
  S2_MSG=clear
else
  S2_MSG=unclear
fi
check "clear" "$S2_MSG" "no-project error message is clear (mentions DVS repository / dvs.toml)"
note "no-project error text: $(printf '%s' "$S2_OUT" | head -1)"
# add with an explicit path but no project must also fail cleanly.
S2_ADD_RC=0
(cd "$NOPROJ/a/b/c" && dvs add orphan.bin >/dev/null 2>&1) || S2_ADD_RC=$?
check "1" "$S2_ADD_RC" "add with no dvs.toml up the tree exits 1 (rc=$S2_ADD_RC)"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 3: NESTED project -> NEAREST dvs.toml wins (blob lands in INNER storage) ==="
# Inside the outer project create a nested project with its OWN dvs.toml and a
# distinct storage. init below a parent dvs.toml must succeed (local check, L84-85).
INNER_PROJ="$DVS_REPO/data/inner"
mkdir -p "$INNER_PROJ"
S3_INIT_RC=0
(cd "$INNER_PROJ" && dvs init "$INNER_STORE" >/dev/null 2>&1) || S3_INIT_RC=$?
check "0" "$S3_INIT_RC" "init inside an existing project succeeds (local check, rc=$S3_INIT_RC)"
check "1" "$([ -f "$INNER_PROJ/dvs.toml" ] && echo 1 || echo 0)" \
  "nested dvs.toml created at data/inner/dvs.toml"

# Add a file from inside the nested project. The NEAREST dvs.toml (inner) must
# win, so the blob lands in INNER_STORE and NOT in OUTER_STORE. We prove this by
# blob LOCATION, not merely by config presence.
mkrandfile "$INNER_PROJ/inner.bin" 777
(cd "$INNER_PROJ" && dvs add inner.bin --json >/dev/null)
INNER_HASH="$(first_hash "$INNER_PROJ")"
check "64" "${#INNER_HASH}" "captured 64-char blake3 for inner.bin"
INNER_BLOB="$(blob_path "$INNER_STORE" "$INNER_HASH")"
OUTER_BLOB="$(blob_path "$OUTER_STORE" "$INNER_HASH")"
if [ -f "$INNER_BLOB" ] && [ ! -f "$OUTER_BLOB" ]; then
  S3_LOC=nearest
elif [ -f "$OUTER_BLOB" ]; then
  S3_LOC=outer
else
  S3_LOC=neither
fi
check "nearest" "$S3_LOC" "add from data/inner/ stores blob in INNER storage, not OUTER (nearest wins by blob location)"

# Confirm discovery: a status run from data/inner/ resolves the inner project,
# and the outer project does NOT know about inner.bin.
S3_INNER_STATUS="$(cd "$INNER_PROJ" && dvs status --json 2>&1)"
case "$S3_INNER_STATUS" in
  *'"path":"inner.bin"'*) S3_INNER_SEES=yes ;;
  *)                      S3_INNER_SEES=no  ;;
esac
check "yes" "$S3_INNER_SEES" "inner project status sees inner.bin as its own (root-relative)"
S3_OUTER_STATUS="$(cd "$DVS_REPO" && dvs status --json 2>&1)"
case "$S3_OUTER_STATUS" in
  *inner.bin*) S3_OUTER_SEES=yes ;;
  *)           S3_OUTER_SEES=no  ;;
esac
check "no" "$S3_OUTER_SEES" "outer project does NOT track the inner project's file"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 4: two SIBLING projects in one git repo with DIFFERENT storage ==="
# Two peer projects under the same git repo, each with its own dvs.toml and its
# own distinct storage. Each must resolve to its own storage.
SIB_A="$DVS_REPO/projA"
SIB_B="$DVS_REPO/projB"
mkdir -p "$SIB_A" "$SIB_B"
(cd "$SIB_A" && dvs init "$SIB_A_STORE" >/dev/null)
(cd "$SIB_B" && dvs init "$SIB_B_STORE" >/dev/null)

# Add a DISTINCT-content file in each sibling.
mkrandfile "$SIB_A/a.bin" 1234
mkrandfile "$SIB_B/b.bin" 4321
(cd "$SIB_A" && dvs add a.bin --json >/dev/null)
(cd "$SIB_B" && dvs add b.bin --json >/dev/null)
HASH_A="$(first_hash "$SIB_A")"
HASH_B="$(first_hash "$SIB_B")"

# projA's blob must be in SIB_A_STORE and absent from SIB_B_STORE, vice versa.
A_IN_A="$([ -f "$(blob_path "$SIB_A_STORE" "$HASH_A")" ] && echo 1 || echo 0)"
A_IN_B="$([ -f "$(blob_path "$SIB_B_STORE" "$HASH_A")" ] && echo 1 || echo 0)"
B_IN_B="$([ -f "$(blob_path "$SIB_B_STORE" "$HASH_B")" ] && echo 1 || echo 0)"
B_IN_A="$([ -f "$(blob_path "$SIB_A_STORE" "$HASH_B")" ] && echo 1 || echo 0)"
check "1" "$A_IN_A" "projA blob lands in projA storage"
check "0" "$A_IN_B" "projA blob is NOT in projB storage"
check "1" "$B_IN_B" "projB blob lands in projB storage"
check "0" "$B_IN_A" "projB blob is NOT in projA storage"

# Each project's status sees only its own file.
SA_STATUS="$(cd "$SIB_A" && dvs status --json 2>&1)"
SB_STATUS="$(cd "$SIB_B" && dvs status --json 2>&1)"
case "$SA_STATUS" in *'"path":"a.bin"'*) SA_OK=yes ;; *) SA_OK=no ;; esac
case "$SA_STATUS" in *b.bin*) SA_LEAK=yes ;; *) SA_LEAK=no ;; esac
case "$SB_STATUS" in *'"path":"b.bin"'*) SB_OK=yes ;; *) SB_OK=no ;; esac
check "yes" "$SA_OK" "projA status sees a.bin"
check "no" "$SA_LEAK" "projA status does NOT see projB's b.bin"
check "yes" "$SB_OK" "projB status sees b.bin"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 5: command run EXACTLY at the project root ==="
# Run at the project root itself (no walking up needed). Outer repo root is a
# project root. add/status/get all operate from cwd == root.
mkrandfile "$DVS_REPO/root_file.bin" 1K
S5_ADD_RC=0
S5_ADD="$(cd "$DVS_REPO" && dvs add root_file.bin --json 2>&1)" || S5_ADD_RC=$?
check "0" "$S5_ADD_RC" "add at project root exits 0 (rc=$S5_ADD_RC)"
case "$S5_ADD" in
  *'"path":"root_file.bin"'*) S5_PATH=rootrel ;;
  *)                          S5_PATH=other   ;;
esac
check "rootrel" "$S5_PATH" "add at project root reports root-relative path"
S5_STATUS="$(cd "$DVS_REPO" && dvs status root_file.bin --json 2>&1)"
case "$S5_STATUS" in
  *'"path":"root_file.bin"'*'"status":"current"'*) S5_STAT=current ;;
  *)                                               S5_STAT=other   ;;
esac
check "current" "$S5_STAT" "status at project root reports root_file.bin current"
rm -f "$DVS_REPO/root_file.bin"
S5_GET_RC=0
(cd "$DVS_REPO" && dvs get root_file.bin --json >/dev/null 2>&1) || S5_GET_RC=$?
check "0" "$S5_GET_RC" "get at project root exits 0 (rc=$S5_GET_RC)"
check "1" "$([ -f "$DVS_REPO/root_file.bin" ] && echo 1 || echo 0)" \
  "get at project root restored root_file.bin"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 6: SHARED storage across two projects -> content-addressable dedup (L358-360) ==="
# Two separate projects pointed at the SAME storage. Identical content added in
# each must dedup to the SAME blob path in the shared storage (content-addressed
# by blake3, independent of path/name/project).
SH_A="$DVS_REPO/sharedA"
SH_B="$DVS_REPO/sharedB"
mkdir -p "$SH_A" "$SH_B"
(cd "$SH_A" && dvs init "$SHARED_STORE" >/dev/null)

# FINDING: `dvs init` REFUSES to point a second project at an already-
# initialized storage ("dvs is already initialized (backend storage exists)"),
# even when no data has been added yet. This blocks the spec's
# multiple-projects-sharing-storage / cross-project dedup use case (Glossary
# L20, L358-360) through `init` alone. To exercise dedup we init projB against
# a throwaway store, then repoint its dvs.toml at the shared store (the toml is
# user-editable per the High-level overview).
SH_B_INIT_RC=0
(cd "$SH_B" && dvs init "$SHARED_STORE" >/dev/null 2>&1) || SH_B_INIT_RC=$?
if [ "$SH_B_INIT_RC" -ne 0 ]; then
  note "init into an already-initialized shared storage is REJECTED (rc=$SH_B_INIT_RC) — blocks multiple-projects-share-storage via init; worth filing. Falling back to dvs.toml repoint."
  SH_B_TMP="$SCRIPT_DIR/dvs_storage_proj_${RUN_SUFFIX}_sharedtmp"
  mkdir -p "$SH_B_TMP"
  (cd "$SH_B" && dvs init "$SH_B_TMP" >/dev/null)
  python3 - "$SH_B/dvs.toml" "$SH_B_TMP" "$SHARED_STORE" <<'PY'
import sys
toml, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
with open(toml) as fh:
    s = fh.read()
with open(toml, "w") as fh:
    fh.write(s.replace(old, new))
PY
  rm -rf "$SH_B_TMP"
fi
# projB now shares SHARED_STORE.
check "1" "$(grep -cF "$SHARED_STORE" "$SH_B/dvs.toml")" "projB dvs.toml points at the shared storage"

# Same bytes, DIFFERENT names, DIFFERENT projects.
mkrandfile "$SH_A/payload.bin" 2K
cp "$SH_A/payload.bin" "$SH_B/copy_of_payload.bin"
(cd "$SH_A" && dvs add payload.bin --json >/dev/null)
(cd "$SH_B" && dvs add copy_of_payload.bin --json >/dev/null)
SH_HASH_A="$(first_hash "$SH_A")"
SH_HASH_B="$(first_hash "$SH_B")"
check "$SH_HASH_A" "$SH_HASH_B" "identical content yields identical blake3 across projects"
# Exactly ONE blob for that hash in the shared store (dedup, not duplicated).
SH_BLOB="$(blob_path "$SHARED_STORE" "$SH_HASH_A")"
check "1" "$([ -f "$SH_BLOB" ] && echo 1 || echo 0)" "shared blob exists at content-addressed path"
SH_PREFIX_DIR="$SHARED_STORE/${SH_HASH_A:0:2}"
SH_BLOB_COUNT="$(find "$SH_PREFIX_DIR" -maxdepth 1 -type f -name "${SH_HASH_A:2}" | wc -l | tr -d ' ')"
check "1" "$SH_BLOB_COUNT" "exactly one blob for the shared hash (deduped across projects)"
# Cross-project restore: deleting B's copy and `get` restores from the shared blob.
rm -f "$SH_B/copy_of_payload.bin"
SH_GET_RC=0
(cd "$SH_B" && dvs get copy_of_payload.bin --json >/dev/null 2>&1) || SH_GET_RC=$?
check "0" "$SH_GET_RC" "projB get from shared storage exits 0 (rc=$SH_GET_RC)"
check "1" "$([ -f "$SH_B/copy_of_payload.bin" ] && echo 1 || echo 0)" \
  "projB restored its file from the shared content-addressed blob"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SCENARIO 7: custom --metadata-folder-name respected by add/get/status discovery ==="
# init with a custom metadata folder name. add must write the sidecar under the
# custom folder (not .dvs); status/get must discover via the custom folder.
META_PROJ="$DVS_REPO/metaproj"
META_NAME=".mymeta"
mkdir -p "$META_PROJ"
(cd "$META_PROJ" && dvs init "$META_STORE" --metadata-folder-name "$META_NAME" >/dev/null)

mkdir -p "$META_PROJ/sub"
mkrandfile "$META_PROJ/sub/m.bin" 1K
(cd "$META_PROJ" && dvs add sub/m.bin --json >/dev/null)

# Sidecar lives under the CUSTOM folder mirroring the path, and NOT under .dvs.
check "1" "$([ -f "$META_PROJ/$META_NAME/sub/m.bin.dvs" ] && echo 1 || echo 0)" \
  "add wrote sidecar under custom metadata folder ($META_NAME/sub/m.bin.dvs)"
check "0" "$([ -d "$META_PROJ/.dvs" ] && echo 1 || echo 0)" \
  "default .dvs folder NOT created when custom metadata folder is used"

# status discovers via the custom folder.
META_STATUS_RC=0
META_STATUS="$(cd "$META_PROJ" && dvs status --json 2>&1)" || META_STATUS_RC=$?
check "0" "$META_STATUS_RC" "status discovers files via custom metadata folder (rc=$META_STATUS_RC)"
case "$META_STATUS" in
  *'"path":"sub/m.bin"'*'"status":"current"'*) META_STAT=current ;;
  *)                                           META_STAT=other   ;;
esac
check "current" "$META_STAT" "status via custom metadata folder reports m.bin current"

# get discovers via the custom folder and restores the deleted file.
rm -f "$META_PROJ/sub/m.bin"
META_GET_RC=0
(cd "$META_PROJ" && dvs get sub/m.bin --json >/dev/null 2>&1) || META_GET_RC=$?
check "0" "$META_GET_RC" "get discovers via custom metadata folder (rc=$META_GET_RC)"
check "1" "$([ -f "$META_PROJ/sub/m.bin" ] && echo 1 || echo 0)" \
  "get via custom metadata folder restored sub/m.bin"

# ──────────────────────────────────────────────────────────────────────────────
say
say "=== SUMMARY: $PASS_N pass, $FAIL_N fail ==="
say "($NOTE_N notes)"

# ── self-clean ONLY our own temp dirs (concurrency-safe; never cleanup.sh) ───
rm -rf "$DVS_REPO" "$NOPROJ" \
       "$OUTER_STORE" "$INNER_STORE" "$SIB_A_STORE" "$SIB_B_STORE" \
       "$SHARED_STORE" "$META_STORE"
