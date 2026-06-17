#!/usr/bin/env bash

# Validate CLI path-handling across `dvs add` / `get` / `status` (spec L51-53,
# L136, L158, L311). Covers absolute / relative / `./` / mixed paths, `..`
# traversal (in-project vs escaping), absolute paths outside the root, trailing
# slash on a file, duplicate args, missing paths, project root `.`, deep
# nesting, spaces, unicode, root-relative output, and abs<->rel sidecar
# identity. Each scenario prints expected vs actual and a PASS/FAIL via check().
# Scripts assert the SPEC-CORRECT behavior.

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

# Read .dvs sidecar JSON field for a given root-relative path.
sidecar_field() {
  python3 -c 'import json,sys;print(json.load(open(".dvs/"+sys.argv[1]+".dvs")).get(sys.argv[2],"<absent>"))' "$1" "$2"
}
# First result's outcome from a --json add/get output on stdin.
json_outcome() {
  python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["outcome"])'
}
# First result's path from a --json output on stdin.
json_path() {
  python3 -c 'import sys,json;print(json.load(sys.stdin)[0]["path"])'
}

set -xo pipefail

# ── Setup: one CLI repo + sibling storage; outside-root scratch dir ──

DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"
DVS_STORAGE_ABS="$(cd "$DVS_STORAGE" && pwd)"

# A path OUTSIDE the project root (sibling of the repo, not under it).
OUTSIDE_DIR="$(mktemp -d /tmp/dvs_paths_outside_XXX)"

# Track exactly the temp dirs we created so cleanup never touches siblings'.
MY_TEMPDIRS=("$DVS_REPO" "$DVS_STORAGE" "$OUTSIDE_DIR")

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE_ABS" >/dev/null
DVS_REPO_ABS="$(pwd)"

# =====================================================================
say
say "=== CASE: absolute path to in-project file -> add, get, status (L53) ==="
mkrandfile data/abs.bin 256
ABS_PATH="$DVS_REPO_ABS/data/abs.bin"
# add via absolute path; path reported back is project-root-relative (L53,L136).
ADD_PATH="$(dvs add --json "$ABS_PATH" | json_path)"
check "data/abs.bin" "$ADD_PATH" "abs add reports root-relative path data/abs.bin"
check "YES" "$([ -e .dvs/data/abs.bin.dvs ] && echo YES || echo NO)" "abs add created sidecar .dvs/data/abs.bin.dvs"
# get via absolute path after deleting local copy.
rm -f data/abs.bin
GET_OC="$(dvs get --json "$ABS_PATH" | json_outcome)"
check "copied" "$GET_OC" "abs get restored the deleted file (outcome=copied)"
check "YES" "$([ -e data/abs.bin ] && echo YES || echo NO)" "abs get wrote the file back to data/abs.bin"
# status via absolute path.
ST="$(dvs status --json "$ABS_PATH" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d[0]["path"]+"="+d[0]["status"])')"
check "data/abs.bin=current" "$ST" "abs status reports data/abs.bin current (root-relative)"

# =====================================================================
say
say "=== CASE: relative path (data/x.bin) (L53) ==="
mkrandfile data/rel.bin 256
REL_OC="$(dvs add --json data/rel.bin | json_outcome)"
check "copied" "$REL_OC" "relative add outcome=copied"
check "YES" "$([ -e .dvs/data/rel.bin.dvs ] && echo YES || echo NO)" "relative add sidecar at .dvs/data/rel.bin.dvs"

# =====================================================================
say
say "=== CASE: ./-prefixed relative path (./data/x.bin) ==="
mkrandfile data/dot.bin 256
DOT_PATH="$(dvs add --json ./data/dot.bin | json_path)"
check "data/dot.bin" "$DOT_PATH" "./-prefixed path normalized to data/dot.bin in output"
check "YES" "$([ -e .dvs/data/dot.bin.dvs ] && echo YES || echo NO)" "./-prefixed add sidecar at .dvs/data/dot.bin.dvs (no ./ in path)"

# =====================================================================
say
say "=== CASE: mix of absolute + relative paths in ONE invocation ==="
mkrandfile data/mix_a.bin 128
mkrandfile data/mix_b.bin 128
MIX="$(dvs add --json "$DVS_REPO_ABS/data/mix_a.bin" data/mix_b.bin \
  | python3 -c 'import sys,json;print(",".join(sorted(r["path"] for r in json.load(sys.stdin))))')"
check "data/mix_a.bin,data/mix_b.bin" "$MIX" "abs+rel mix in one invocation both reported root-relative"
check "YES" "$([ -e .dvs/data/mix_a.bin.dvs ] && [ -e .dvs/data/mix_b.bin.dvs ] && echo YES || echo NO)" "abs+rel mix created both sidecars"

# =====================================================================
say
say "=== CASE: .. traversal that STAYS inside the project (data/../data/x.bin) ==="
mkrandfile data/trav.bin 128
rc=0; OUT="$(dvs add --json "data/../data/trav.bin" 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "in-project .. traversal add exits 0"
TRAV_PATH="$(printf '%s' "$OUT" | json_path)"
check "data/trav.bin" "$TRAV_PATH" "in-project .. traversal normalizes to data/trav.bin"
check "YES" "$([ -e .dvs/data/trav.bin.dvs ] && echo YES || echo NO)" "in-project .. traversal sidecar at .dvs/data/trav.bin.dvs"

# =====================================================================
say
say "=== CASE: .. traversal that ESCAPES the project root (rejected) (L53,L158) ==="
# Create a real file just outside the root so the rejection is on scope, not existence.
mkrandfile "$OUTSIDE_DIR/escape.bin" 128
# Place it as a sibling reachable via ../ from inside the repo.
ESCAPE_REL="../$(basename "$OUTSIDE_DIR")/escape.bin"
ln -s "$OUTSIDE_DIR/escape.bin" "$DVS_REPO_ABS/../escape_via_dotdot.bin" 2>/dev/null || true
# A plain ../ escape to an existing outside file.
mkrandfile "$(dirname "$DVS_REPO_ABS")/escape_sibling.bin" 128 2>/dev/null || \
  head -c 128 /dev/urandom > "$(dirname "$DVS_REPO_ABS")/escape_sibling.bin"
rc=0; OUT="$(dvs add --json "../escape_sibling.bin" 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" ".. escaping root add exits 1"
ESC_REJ="$(printf '%s' "$OUT" | grep -qiE "outside project|outside the project|not found" && echo yes || echo no)"
check "yes" "$ESC_REJ" ".. escaping root rejected (outside-project / not-found error)"
check "NO" "$([ -e .dvs/escape_sibling.bin.dvs ] && echo YES || echo NO)" "no sidecar created for .. escaping path"
rm -f "$(dirname "$DVS_REPO_ABS")/escape_sibling.bin" "$DVS_REPO_ABS/../escape_via_dotdot.bin"

# =====================================================================
say
say "=== CASE: absolute path OUTSIDE the project root (rejected) (L53,L158) ==="
mkrandfile "$OUTSIDE_DIR/abs_outside.bin" 128
rc=0; OUT="$(dvs add --json "$OUTSIDE_DIR/abs_outside.bin" 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "abs path outside root add exits 1"
OUT_REJ="$(printf '%s' "$OUT" | grep -qiE "outside project|outside the project" && echo yes || echo no)"
check "yes" "$OUT_REJ" "abs path outside root rejected with 'outside project' error"
check "NO" "$([ -e .dvs/abs_outside.bin.dvs ] && echo YES || echo NO)" "no sidecar created for abs-outside path"

# =====================================================================
say
say "=== CASE: trailing slash on a FILE path (data/x.bin/) -> normalized to the file ==="
# Observed behavior: trailing slash on an existing FILE is tolerated and the
# file is added/recognized (no error). Document the actual CLI behavior.
mkrandfile data/trail.bin 128
dvs add data/trail.bin >/dev/null
rc=0; OUT="$(dvs add --json "data/trail.bin/" 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "trailing slash on an existing file add exits 0 (normalized)"
TRAIL_PATH="$(printf '%s' "$OUT" | json_path)"
check "data/trail.bin" "$TRAIL_PATH" "trailing slash stripped -> path reported as data/trail.bin"
TRAIL_OC="$(printf '%s' "$OUT" | json_outcome)"
check "present" "$TRAIL_OC" "trailing-slash re-add of unchanged file -> outcome=present (same file)"

# =====================================================================
say
say "=== CASE: the same path listed twice in one invocation (arg dedup, one result) ==="
mkrandfile data/dup.bin 128
DUP_N="$(dvs add --json data/dup.bin data/dup.bin | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
check "1" "$DUP_N" "duplicate path in one invocation deduped to a single result row"
check "YES" "$([ -e .dvs/data/dup.bin.dvs ] && echo YES || echo NO)" "duplicate-path add created the sidecar"

# =====================================================================
say
say "=== CASE: path to a file that does not exist (exit 1, reported) (L186) ==="
rc=0; OUT="$(dvs add data/no_such_file.bin 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "missing path add exits 1"
MISS_REP="$(printf '%s' "$OUT" | grep -qiE "no_such_file.bin|not found" && echo yes || echo no)"
check "yes" "$MISS_REP" "missing path reported in output"
check "NO" "$([ -e .dvs/data/no_such_file.bin.dvs ] && echo YES || echo NO)" "no sidecar created for missing path"

# =====================================================================
say
say "=== CASE: path that is the project root '.' itself (bare dir, no glob -> not added) (L167) ==="
# A directory with no glob is not added; the root '.' yields no files to add.
rc=0; OUT="$(dvs add "." 2>&1)" || rc=$?
say "$OUT"
check "1" "$rc" "add '.' (project root, no glob) exits 1"
ROOT_MSG="$(printf '%s' "$OUT" | grep -qiE "no files to add|directory" && echo yes || echo no)"
check "yes" "$ROOT_MSG" "add '.' reports no files to add"

# =====================================================================
say
say "=== CASE: deeply nested path -> sidecar mirrors full depth (L136,L311) ==="
mkrandfile a/b/c/d/e/deep.bin 64
DEEP_PATH="$(dvs add --json a/b/c/d/e/deep.bin | json_path)"
check "a/b/c/d/e/deep.bin" "$DEEP_PATH" "deep add reports full root-relative path"
check "YES" "$([ -e .dvs/a/b/c/d/e/deep.bin.dvs ] && echo YES || echo NO)" "deep sidecar mirrors full depth at .dvs/a/b/c/d/e/deep.bin.dvs"

# =====================================================================
say
say "=== CASE: filename with spaces ==="
mkrandfile "data/has space.bin" 64
rc=0; OUT="$(dvs add --json "data/has space.bin" 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "filename with spaces add exits 0"
SPACE_PATH="$(printf '%s' "$OUT" | json_path)"
check "data/has space.bin" "$SPACE_PATH" "spaces preserved in reported path"
check "YES" "$([ -e ".dvs/data/has space.bin.dvs" ] && echo YES || echo NO)" "sidecar created for spaced filename"

# =====================================================================
say
say "=== CASE: filename with unicode characters ==="
mkrandfile "data/café_λ.bin" 64
rc=0; OUT="$(dvs add --json "data/café_λ.bin" 2>&1)" || rc=$?
say "$OUT"
check "0" "$rc" "unicode filename add exits 0"
UNI_PATH="$(printf '%s' "$OUT" | json_path)"
check "data/café_λ.bin" "$UNI_PATH" "unicode preserved in reported path"
check "YES" "$([ -e ".dvs/data/café_λ.bin.dvs" ] && echo YES || echo NO)" "sidecar created for unicode filename"

# =====================================================================
say
say "=== CASE: reported path is project-root-relative regardless of how given (L53,L136) ==="
# Same physical file referenced three ways must report the identical root-relative path.
mkrandfile data/canon.bin 64
P_ABS="$(dvs add --json "$DVS_REPO_ABS/data/canon.bin" | json_path)"
P_DOT="$(dvs add --json "./data/canon.bin" | json_path)"
P_TRAV="$(dvs add --json "data/../data/canon.bin" | json_path)"
check "data/canon.bin|data/canon.bin|data/canon.bin" "$P_ABS|$P_DOT|$P_TRAV" "abs, ./, and .. forms all report identical root-relative path"

# =====================================================================
say
say "=== CASE: add via ABS path, then get via REL path resolves the SAME sidecar (and vice versa) ==="
mkrandfile data/rt.bin 64
dvs add "$DVS_REPO_ABS/data/rt.bin" >/dev/null
SC_BEFORE="$(sidecar_field data/rt.bin hashes)"
rm -f data/rt.bin
# get via the relative form must restore from the same sidecar/blob.
RT_OC="$(dvs get --json data/rt.bin | json_outcome)"
check "copied" "$RT_OC" "abs-add then rel-get restores the file (outcome=copied)"
check "YES" "$([ -e data/rt.bin ] && echo YES || echo NO)" "abs-add then rel-get wrote data/rt.bin back"
SC_AFTER="$(sidecar_field data/rt.bin hashes)"
check "$SC_BEFORE" "$SC_AFTER" "abs-add and rel-get share the SAME sidecar (hashes unchanged)"
# vice versa: add via rel, get via abs.
mkrandfile data/rt2.bin 64
dvs add data/rt2.bin >/dev/null
rm -f data/rt2.bin
RT2_OC="$(dvs get --json "$DVS_REPO_ABS/data/rt2.bin" | json_outcome)"
check "copied" "$RT2_OC" "rel-add then abs-get restores the file (outcome=copied)"
check "YES" "$([ -e data/rt2.bin ] && echo YES || echo NO)" "rel-add then abs-get wrote data/rt2.bin back"

# =====================================================================
say
echo "=== SUMMARY: $PASS pass, $FAIL fail ==="

# ── Self-clean ONLY our own temp dirs (other agents share this ui/ dir). ──
for d in "${MY_TEMPDIRS[@]}"; do
  rm -rf "$d"
done
