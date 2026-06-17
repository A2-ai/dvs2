#!/usr/bin/env bash

# Exhaustive globbing scenarios for add / get / status (cases_glob.sh in
# ui/SCENARIOS.md). Superset of validate_glob.sh. CLI only; the dvs binary is
# already installed from this worktree (do not reinstall).
#
# Spec under test (specs.md):
#   Globbing section (L447-456):
#     - explicit files: glob ignored, files used directly (L451)
#     - explicit directories + glob: walked and filtered (L452)
#     - no paths + glob: walks current directory filtered (L453)
#     - literal path separator: '*.bin' matches only the target dir, NOT
#       subdirs; '**/*.bin' matches recursively (L455-456)
#   add  (L167): paths or --glob; dir + glob runs glob from that dir
#   get  (L221,L228): same rules, resolved against the metadata folder;
#                     '**/*' restores every tracked file
#   status: per SCENARIOS legend status now ACCEPTS --glob/-g on this branch
#
# Tracked divergences to LABEL (verify, do not assume):
#   #217 get <dir> --glob (positional directory)
#   #218 status --glob / -g
#
# IMPORTANT: every glob passed to --glob/-g is QUOTED so the shell does not
# expand it; the spec requires lib-side expansion. One deliberate UNQUOTED case
# is included to contrast shell-expansion (add only, per L184).

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# ── Verdict tracking ───────────────────────────────────────────────────────
PASS=0
FAIL=0
NOTE=0
check() {
  { set +x; } 2>/dev/null
  # check WANT GOT LABEL
  if [ "$1" = "$2" ]; then
    echo "PASS: $3"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $3 (want=[$1] got=[$2])"
    FAIL=$((FAIL + 1))
  fi
  { set -x; } 2>/dev/null
}

# A non-failing observation (behavior recorded, no spec assertion).
note() {
  { set +x; } 2>/dev/null
  echo "NOTE: $1"
  NOTE=$((NOTE + 1))
  { set -x; } 2>/dev/null
}

# Sorted, comma-joined list of "path" values from a --json result array.
json_paths() {
  { set +x; } 2>/dev/null
  grep -oE '"path":"[^"]*"' | sed 's/"path":"//;s/"$//' | sort | paste -sd, -
  { set -x; } 2>/dev/null
}

# Sorted, comma-joined list of tracked sidecars relative to the .dvs folder.
sidecar_paths() {
  { set +x; } 2>/dev/null
  ( cd .dvs && find . -name '*.dvs' -type f \
      | sed 's#^\./##;s#\.dvs$##' | sort | paste -sd, - )
  { set -x; } 2>/dev/null
}

# Wipe tracked metadata so sidecar sets are unambiguous between scenarios.
reset_repo() {
  { set +x; } 2>/dev/null
  find .dvs -name '*.dvs' -delete 2>/dev/null || true
  { set -x; } 2>/dev/null
}

# Remove the local copies of the *.bin fixture so get has work to do.
del_locals() {
  { set +x; } 2>/dev/null
  rm -f top1.bin top2.bin sub/s1.bin sub/s2.bin data/d1.bin data/d2.bin
  { set -x; } 2>/dev/null
}

# ── Setup: temp repo + sibling storage (outside the repo) ──────────────────
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

# Clean up ONLY our own dirs on exit (concurrency-safe; never cleanup.sh).
trap 'rm -rf "$DVS_REPO" "$DVS_STORAGE"' EXIT

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# ── Fixture: top-level *.bin + sub/*.bin + data/*.bin + non-.bin files ─────
# Layout:
#   top1.bin top2.bin            (top-level *.bin)
#   sub/s1.bin sub/s2.bin        (nested *.bin)
#   data/d1.bin data/d2.bin      (nested *.bin in another dir)
#   keep1.txt keep2.txt          (explicit non-glob files, top level)
say
say "=== fixture ==="
mkfiles 2 256 .          # creates file_1.bin file_2.bin at top level
mv file_1.bin top1.bin
mv file_2.bin top2.bin
mkfiles 2 256 sub
mv sub/file_1.bin sub/s1.bin
mv sub/file_2.bin sub/s2.bin
mkfiles 2 256 data
mv data/file_1.bin data/d1.bin
mv data/file_2.bin data/d2.bin
printf 'alpha\n' > keep1.txt
printf 'beta\n'  > keep2.txt
tree --noreport -I '.git'

# Expected matched-file sets for the *.bin fixture:
TOP_ONLY="top1.bin,top2.bin"
ALL_BIN="data/d1.bin,data/d2.bin,sub/s1.bin,sub/s2.bin,top1.bin,top2.bin"
DATA_DIR_BIN="data/d1.bin,data/d2.bin"
EXPLICIT_TXT="keep1.txt,keep2.txt"
SUB_DIR_BIN="sub/s1.bin,sub/s2.bin"

# ═══════════════════════════════════════════════════════════════════════════
# ADD scenarios
# ═══════════════════════════════════════════════════════════════════════════

# ── add: literal separator '*.bin' -> top-level only (L455) ──
say
say "=== ADD: --glob '*.bin' literal separator (top-level only) ==="
out="$(dvs add --glob '*.bin' --json)"
got="$(printf '%s' "$out" | json_paths)"
check "$TOP_ONLY" "$got" "add --glob '*.bin' matches ONLY target dir (not subdirs)"
check "$TOP_ONLY" "$(sidecar_paths)" "add '*.bin' wrote sidecars only for top-level"
reset_repo

# ── add: '**/*.bin' recursive across subdirs (L456) ──
say
say "=== ADD: --glob '**/*.bin' recursive ==="
out="$(dvs add --glob '**/*.bin' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "add --glob '**/*.bin' matches recursively"
check "$ALL_BIN" "$(sidecar_paths)" "add '**/*.bin' wrote sidecars for all *.bin recursively"
reset_repo

# ── add: explicit directory + glob -> dir walked, filtered (L452, L167) ──
say
say "=== ADD: explicit dir 'data' + --glob '*.bin' ==="
out="$(dvs add data --glob '*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "add data --glob '*.bin' walks data/ filtered by glob"
reset_repo

# ── add: dir baked into pattern 'data/*.bin' (no positional dir) ──
say
say "=== ADD: --glob 'data/*.bin' (dir baked into pattern) ==="
out="$(dvs add --glob 'data/*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "add --glob 'data/*.bin' matches just data/ via pattern"
reset_repo

# ── add: explicit files -> glob ignored (L451) ──
say
say "=== ADD: explicit files keep1.txt keep2.txt + --glob '*.bin' (glob ignored) ==="
out="$(dvs add keep1.txt keep2.txt --glob '*.bin' --json)"
check "$EXPLICIT_TXT" "$(printf '%s' "$out" | json_paths)" "add explicit files ignores --glob, adds exact files"
reset_repo

# ── add: no paths + glob -> walks cwd filtered (L453) ──
say
say "=== ADD: no paths + --glob '*.bin' walks current dir ==="
out="$(dvs add --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "add (no paths) --glob walks cwd filtered by glob"
reset_repo

# ── add: glob matching ZERO files -> error, exit 1 ──
say
say "=== ADD: --glob '*.nomatch' matches nothing (error + exit 1) ==="
rc=0
out="$(dvs add --glob '*.nomatch' --json 2>&1)" || rc=$?
say "add zero-match exit=$rc output=$out"
check "1" "$rc" "add zero-match glob exits 1"
check "" "$(printf '%s' "$out" | json_paths)" "add zero-match glob produced no matched paths"
reset_repo

# ── add: glob run from a SUBDIRECTORY (cwd-relative) ──
# From inside sub/, '*.bin' must match sub/s1.bin sub/s2.bin only. Paths in the
# JSON are reported project-root-relative.
say
say "=== ADD: from subdir 'sub', --glob '*.bin' (cwd-relative walk) ==="
out="$(cd sub && dvs add --glob '*.bin' --json)"
check "$SUB_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "add from sub/ --glob '*.bin' walks sub/ (cwd-relative)"
check "$SUB_DIR_BIN" "$(sidecar_paths)" "add from sub/ wrote sidecars only for sub/*.bin"
reset_repo

# ── add: unquoted *.bin (shell-expanded) vs quoted '*.bin' (lib-expanded) ──
# Both must yield the SAME top-level result set. Unquoted is expanded by the
# shell into explicit paths top1.bin top2.bin (top level only); quoted is
# expanded by the library with the literal-separator rule (top level only).
say
say "=== ADD: unquoted *.bin (shell) vs quoted '*.bin' (lib) -> same set ==="
out_unq="$(dvs add *.bin --json)"
unq="$(printf '%s' "$out_unq" | json_paths)"
check "$TOP_ONLY" "$unq" "add *.bin (shell-expanded, unquoted) adds top-level matches"
reset_repo
out_q="$(dvs add --glob '*.bin' --json)"
q="$(printf '%s' "$out_q" | json_paths)"
check "$unq" "$q" "add quoted '*.bin' == unquoted *.bin result set"
reset_repo

# ═══════════════════════════════════════════════════════════════════════════
# Seed full metadata so GET / STATUS have all 6 *.bin to resolve against.
# ═══════════════════════════════════════════════════════════════════════════
say
say "=== seed: add --glob '**/*.bin' so all 6 *.bin are tracked ==="
dvs add --glob '**/*.bin' --json | json_paths
check "$ALL_BIN" "$(sidecar_paths)" "seed: all 6 *.bin sidecars present for get/status tests"

# ═══════════════════════════════════════════════════════════════════════════
# GET scenarios (get uses -g/--glob; resolves against metadata folder, L221)
# ═══════════════════════════════════════════════════════════════════════════

# ── get: literal separator '*.bin' -> top-level only ──
say
say "=== GET: --glob '*.bin' literal separator (top-level only) ==="
del_locals
out="$(dvs get --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get --glob '*.bin' matches ONLY target dir (not subdirs)"

# ── get: '**/*.bin' recursive (short flag -g) ──
say
say "=== GET: -g '**/*.bin' recursive (short flag) ==="
del_locals
out="$(dvs get -g '**/*.bin' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "get -g '**/*.bin' matches recursively (short flag works)"

# ── get: '**/*' restores everything (L228) ──
say
say "=== GET: --glob '**/*' restores every tracked file (L228) ==="
del_locals
out="$(dvs get --glob '**/*' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "get --glob '**/*' restores all tracked files"

# ── get: explicit files -> glob ignored ──
say
say "=== GET: explicit files top1.bin top2.bin + --glob (glob ignored) ==="
del_locals
out="$(dvs get top1.bin top2.bin --glob '**/*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get explicit files ignores --glob, retrieves exact files"

# ── get: explicit dir + glob -> dir walked filtered (#217) ──
# #217 history: get with positional directory + glob was previously broken
# ("No files to get", exit 1) while add data --glob worked. Verify on THIS
# branch; label the result.
say
say "=== GET: #217 explicit dir 'data' + --glob '*.bin' (positional dir) ==="
del_locals
rc=0
out="$(dvs get data --glob '*.bin' --json 2>&1)" || rc=$?
say "get data --glob '*.bin' exit=$rc output=$out"
got="$(printf '%s' "$out" | json_paths)"
if [ "$got" = "$DATA_DIR_BIN" ] && [ "$rc" = "0" ]; then
  check "$DATA_DIR_BIN" "$got" "#217 get <dir> --glob positional WORKS (walks data/ filtered) -> FIXED"
else
  check "$DATA_DIR_BIN" "<exit $rc / $got>" "#217 get <dir> --glob positional still DIVERGENT (CLI-WRONG vs L452/L221)"
fi

# Contrast: directory baked into the glob string works for get.
say
say "=== GET: --glob 'data/*.bin' (dir in glob string) ==="
del_locals
out="$(dvs get --glob 'data/*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "get --glob 'data/*.bin' (dir in glob string) matches data/"

# ── get: no paths + glob -> walks metadata filtered (L221) ──
say
say "=== GET: no paths + --glob '*.bin' (metadata-relative) ==="
del_locals
out="$(dvs get --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get (no paths) --glob walks metadata filtered by glob"

# ── get: glob matching ZERO files -> error, exit 1 ──
say
say "=== GET: --glob '*.nomatch' matches nothing (error + exit 1) ==="
rc=0
out="$(dvs get --glob '*.nomatch' --json 2>&1)" || rc=$?
say "get zero-match exit=$rc output=$out"
check "1" "$rc" "get zero-match glob exits 1"
check "" "$(printf '%s' "$out" | json_paths)" "get zero-match glob produced no matched paths"

# ── get: glob run from a SUBDIRECTORY ──
# get resolves against the metadata folder, not cwd. Document what running from
# sub/ with '*.bin' actually returns.
say
say "=== GET: from subdir 'sub', --glob '*.bin' (metadata vs cwd) ==="
del_locals
rc=0
out="$(cd sub && dvs get --glob '*.bin' --json 2>&1)" || rc=$?
sub_got="$(printf '%s' "$out" | json_paths)"
say "get from sub/ --glob '*.bin' exit=$rc paths=$sub_got"
if [ "$sub_got" = "$SUB_DIR_BIN" ]; then
  check "$SUB_DIR_BIN" "$sub_got" "get from sub/ --glob '*.bin' resolves sub/*.bin (cwd-relative against metadata)"
else
  note "get from sub/ --glob '*.bin' exit=$rc paths=[$sub_got] (metadata-relative, not cwd-relative — document)"
fi

# ── get: glob + explicit paths together ──
say
say "=== GET: explicit file top1.bin + --glob 'data/*.bin' together ==="
del_locals
rc=0
out="$(dvs get top1.bin --glob 'data/*.bin' --json 2>&1)" || rc=$?
got="$(printf '%s' "$out" | json_paths)"
say "get top1.bin --glob 'data/*.bin' exit=$rc paths=$got"
# Per L451 explicit paths take precedence and glob is ignored -> top1.bin only.
if [ "$got" = "top1.bin" ]; then
  check "top1.bin" "$got" "get explicit file + glob ignores glob, retrieves explicit only"
else
  note "get explicit file + glob exit=$rc paths=[$got] (glob NOT ignored when explicit path present — document)"
fi

# restore everything for status tests
del_locals
dvs get -g '**/*.bin' --json >/dev/null

# ═══════════════════════════════════════════════════════════════════════════
# STATUS scenarios — #218.
# On THIS branch status accepts --glob/-g (previously rejected, exit 2).
# Verify it (a) accepts the flag, (b) resolves against the metadata folder,
# (c) honors the literal-separator + recursive rules.
# ═══════════════════════════════════════════════════════════════════════════
say
say "=== STATUS: #218 does --glob exit 0 (accepted) at all? ==="
rc=0
status_out="$(dvs status --glob '*.bin' --json 2>&1)" || rc=$?
say "status --glob '*.bin' exit=$rc"
check "0" "$rc" "#218 status --glob is ACCEPTED (exit 0) on this branch"

# status path values from the metadata-shaped JSON (same "path":"..." key).
say
say "=== STATUS: #218 --glob '*.bin' literal separator (top-level only) ==="
got="$(printf '%s' "$status_out" | json_paths)"
check "$TOP_ONLY" "$got" "#218 status --glob '*.bin' follows literal-separator rule (top-level only)"

say
say "=== STATUS: #218 -g short flag accepted ==="
rc=0
status_out_g="$(dvs status -g '*.bin' --json 2>&1)" || rc=$?
check "0" "$rc" "#218 status -g short flag ACCEPTED (exit 0)"
check "$TOP_ONLY" "$(printf '%s' "$status_out_g" | json_paths)" "#218 status -g '*.bin' == --glob '*.bin' result set"

say
say "=== STATUS: #218 --glob '**/*.bin' recursive across subdirs ==="
rc=0
status_rec="$(dvs status --glob '**/*.bin' --json 2>&1)" || rc=$?
check "0" "$rc" "status --glob '**/*.bin' exit 0"
check "$ALL_BIN" "$(printf '%s' "$status_rec" | json_paths)" "#218 status --glob '**/*.bin' matches recursively (all tracked)"

say
say "=== STATUS: --glob 'data/*.bin' (dir baked into pattern) ==="
rc=0
status_data="$(dvs status --glob 'data/*.bin' --json 2>&1)" || rc=$?
check "0" "$rc" "status --glob 'data/*.bin' exit 0"
check "$DATA_DIR_BIN" "$(printf '%s' "$status_data" | json_paths)" "status --glob 'data/*.bin' matches just data/"

say
say "=== STATUS: --glob '*.nomatch' matches nothing (empty + exit?) ==="
rc=0
status_zero="$(dvs status --glob '*.nomatch' --json 2>&1)" || rc=$?
say "status zero-match exit=$rc output=$status_zero"
check "" "$(printf '%s' "$status_zero" | json_paths)" "status zero-match glob yields empty result set"
note "status zero-match glob exit code = $rc (status does not treat empty as error)"

# ── status: explicit dir 'data' + --glob '*.bin' (positional dir) ──
say
say "=== STATUS: explicit dir 'data' + --glob '*.bin' (positional dir) ==="
rc=0
status_dir="$(dvs status data --glob '*.bin' --json 2>&1)" || rc=$?
got="$(printf '%s' "$status_dir" | json_paths)"
say "status data --glob '*.bin' exit=$rc paths=$got"
if [ "$got" = "$DATA_DIR_BIN" ] && [ "$rc" = "0" ]; then
  check "$DATA_DIR_BIN" "$got" "status <dir> --glob positional walks data/ filtered"
else
  note "status data --glob '*.bin' exit=$rc paths=[$got] (positional dir + glob behavior — document)"
fi

# ── status: glob run from a SUBDIRECTORY ──
say
say "=== STATUS: from subdir 'sub', --glob '*.bin' ==="
rc=0
status_sub="$(cd sub && dvs status --glob '*.bin' --json 2>&1)" || rc=$?
sub_got="$(printf '%s' "$status_sub" | json_paths)"
say "status from sub/ --glob '*.bin' exit=$rc paths=$sub_got"
if [ "$sub_got" = "$SUB_DIR_BIN" ]; then
  check "$SUB_DIR_BIN" "$sub_got" "status from sub/ --glob '*.bin' resolves sub/*.bin"
else
  note "status from sub/ --glob '*.bin' exit=$rc paths=[$sub_got] (metadata- vs cwd-relative — document)"
fi

# ═══════════════════════════════════════════════════════════════════════════
say
say "=== SUMMARY: $PASS pass, $FAIL fail, $NOTE note ==="
