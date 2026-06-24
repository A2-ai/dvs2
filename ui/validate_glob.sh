#!/usr/bin/env bash

# Validate the Globbing claims in specs.md (L429-438) plus the glob mentions in
# add (L170-171), get (L208), and status. CLI only.
#
# Claims under test (Script 5 in ui/TODO.md):
#   1. explicit files: glob ignored, files added/retrieved directly (L433)
#   2. explicit directory + glob: directory walked, filtered by glob (L434)
#   3. no paths + glob: walks current directory filtered by glob (L435)
#   4. literal separator: '*.bin' matches only target dir, NOT subdir (L437-438)
#   5. '**/*.bin' matches recursively across subdirs (L438)
#   6. same glob rules hold for add, status, AND get (L431)
#
# IMPORTANT: every glob passed to --glob/-g is QUOTED so the shell does not
# expand it; the spec requires lib-side expansion. One deliberate UNQUOTED case
# is included to contrast shell-expansion (add only, per L170).

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

# ── Verdict tracking ───────────────────────────────────────────────────────
PASS=0
FAIL=0
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

# Sorted, comma-joined list of "path" values from a --json result array.
json_paths() {
  { set +x; } 2>/dev/null
  # reads JSON on stdin, extracts every "path":"..." value, sorts, joins on ','
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

# ── Setup: temp repo + sibling storage (outside the repo) ──────────────────
DVS_REPO="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX)"
RUN_SUFFIX="${DVS_REPO##*_}"
DVS_STORAGE="$SCRIPT_DIR/dvs_storage_cli_$RUN_SUFFIX"
mkdir "$DVS_STORAGE"

cd "$DVS_REPO"
git init -q
dvs init "$DVS_STORAGE"

# ── Fixture: top-level *.bin + subdir/*.bin + data/*.bin + non-.bin files ──
# Layout:
#   top1.bin top2.bin            (top-level *.bin)
#   sub/s1.bin sub/s2.bin        (nested *.bin)
#   data/d1.bin data/d2.bin      (nested *.bin in another dir)
#   keep1.txt keep2.txt          (explicit non-glob files)
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

# ═══════════════════════════════════════════════════════════════════════════
# ADD claims
# ═══════════════════════════════════════════════════════════════════════════

# ── Claim 4 (add): literal separator '*.bin' -> top-level only (L437-438) ──
say
say "=== ADD claim 4: --glob '*.bin' literal separator (top-level only) ==="
out="$(dvs add --glob '*.bin' --json)"
got="$(printf '%s' "$out" | json_paths)"
check "$TOP_ONLY" "$got" "add --glob '*.bin' matches ONLY target dir (not subdir)"
# sidecars must mirror exactly the matched set
check "$TOP_ONLY" "$(sidecar_paths)" "add '*.bin' wrote sidecars only for top-level"

# reset repo (fresh metadata) between scenarios so sidecar sets are unambiguous
reset_repo() {
  { set +x; } 2>/dev/null
  rm -rf .dvs/*.dvs .dvs/*/ 2>/dev/null || true
  find .dvs -name '*.dvs' -delete 2>/dev/null || true
  { set -x; } 2>/dev/null
}

reset_repo

# ── Claim 5 (add): '**/*.bin' recursive (L438) ──
say
say "=== ADD claim 5: --glob '**/*.bin' recursive ==="
out="$(dvs add --glob '**/*.bin' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "add --glob '**/*.bin' matches recursively"
check "$ALL_BIN" "$(sidecar_paths)" "add '**/*.bin' wrote sidecars for all *.bin recursively"

reset_repo

# ── Claim 2 (add): explicit directory + glob -> dir walked, filtered (L434) ──
say
say "=== ADD claim 2: explicit dir 'data' + --glob '*.bin' ==="
out="$(dvs add data --glob '*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "add data --glob '*.bin' walks data/ filtered by glob"

reset_repo

# ── Claim 1 (add): explicit files -> glob ignored (L433) ──
say
say "=== ADD claim 1: explicit files keep1.txt keep2.txt + --glob '*.bin' (glob ignored) ==="
out="$(dvs add keep1.txt keep2.txt --glob '*.bin' --json)"
check "$EXPLICIT_TXT" "$(printf '%s' "$out" | json_paths)" "add explicit files ignores --glob, adds exact files"

reset_repo

# ── Claim 3 (add): no paths + glob -> walks cwd filtered (L435) ──
# (Same invocation form as claim 4/5; assert it walked the CURRENT dir.)
say
say "=== ADD claim 3: no paths, --glob '*.bin' walks current dir ==="
out="$(dvs add --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "add (no paths) --glob walks cwd filtered by glob"

reset_repo

# ── add bonus: UNQUOTED *.bin is expanded by the SHELL (L170) ──
say
say "=== ADD bonus: unquoted *.bin (shell expansion, L170) ==="
# Shell expands *.bin at top level only -> top1.bin top2.bin as explicit paths.
out="$(dvs add *.bin --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "add *.bin (shell-expanded) adds top-level matches"

# ═══════════════════════════════════════════════════════════════════════════
# Populate full metadata so GET/STATUS have something to resolve against.
# ═══════════════════════════════════════════════════════════════════════════
reset_repo
say
say "=== seed: add --glob '**/*.bin' so all 6 *.bin are tracked ==="
dvs add --glob '**/*.bin' --json | json_paths
check "$ALL_BIN" "$(sidecar_paths)" "seed: all 6 *.bin sidecars present for get/status tests"

# ═══════════════════════════════════════════════════════════════════════════
# GET claims (get uses -g/--glob; resolves against metadata folder, L208)
# ═══════════════════════════════════════════════════════════════════════════

del_locals() {
  { set +x; } 2>/dev/null
  rm -f top1.bin top2.bin sub/s1.bin sub/s2.bin data/d1.bin data/d2.bin
  { set -x; } 2>/dev/null
}

# ── Claim 4 (get): literal separator '*.bin' -> top-level only ──
say
say "=== GET claim 4: --glob '*.bin' literal separator (top-level only) ==="
del_locals
out="$(dvs get --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get --glob '*.bin' matches ONLY target dir (not subdir)"

# ── Claim 5 (get): '**/*.bin' recursive ──
say
say "=== GET claim 5: -g '**/*.bin' recursive (short flag) ==="
del_locals
out="$(dvs get -g '**/*.bin' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "get -g '**/*.bin' matches recursively (short flag works)"

# ── Claim 1 (get): explicit files -> glob ignored ──
say
say "=== GET claim 1: explicit files + --glob (glob ignored) ==="
del_locals
out="$(dvs get top1.bin top2.bin --glob '**/*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get explicit files ignores --glob, retrieves exact files"

# ── Claim 2 (get): explicit dir + glob -> dir walked filtered (#217 fixed) ──
# Spec L208: get glob resolution works the same way as add; L434: an explicit
# directory arg is walked and filtered by the glob. (#217, which had get return
# "No files to get" for the positional-dir form, is fixed.)
say
say "=== GET claim 2: explicit dir 'data' + --glob '*.bin' ==="
del_locals
out="$(dvs get data --glob '*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "get data --glob '*.bin' walks data/ filtered by glob (#217)"
# Contrast: directory inside the glob string works for get.
del_locals
out="$(dvs get --glob 'data/*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "get --glob 'data/*.bin' (dir in glob string) DOES work"

# ── Claim 3 (get): no paths + glob -> walks cwd filtered ──
say
say "=== GET claim 3: no paths + --glob '*.bin' ==="
del_locals
out="$(dvs get --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "get (no paths) --glob walks (metadata) filtered by glob"

# restore everything
dvs get -g '**/*.bin' --json >/dev/null

# ═══════════════════════════════════════════════════════════════════════════
# STATUS glob claims (spec L431 lists status among --glob commands)
# ═══════════════════════════════════════════════════════════════════════════
say
say "=== STATUS: accepts --glob and follows the literal-separator rule (spec L431, #218) ==="
rc=0
status_out="$(dvs status --glob '*.bin' --json 2>&1)" || rc=$?
say "status --glob exit code: $rc"
say "status --glob stderr/stdout: $status_out"
# Spec L431: add, status, get all accept --glob. (#218, which had status reject
# the flag, is fixed.) The literal `*` does not cross directory separators, so
# '*.bin' matches only top-level .bin files.
check "0" "$rc" "status --glob is accepted (exit 0), not rejected (#218)"
got="$(printf '%s' "$status_out" | json_paths)"
check "$TOP_ONLY" "$got" "status --glob '*.bin' follows literal-separator rule (top-level only)"

# -g is the short form of --glob.
say
say "=== STATUS: -g short flag is accepted ==="
rc=0
dvs status -g '*.bin' >/dev/null 2>&1 || rc=$?
check "0" "$rc" "status -g accepted (exit 0), short form of --glob (#218)"

# ═══════════════════════════════════════════════════════════════════════════
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
