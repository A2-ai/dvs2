#!/usr/bin/env bash

# Validate the Globbing claims in specs.md plus the glob mentions in
# add, get, and status. CLI only.
#
# Claims under test (Script 5 in ui/TODO.md):
#   1. explicit files only + glob: refused, invalid_arguments, exit 2.
#      Files and a directory: the files as given, the glob searches the
#      directory. An unquoted glob the shell expanded gets a hint to quote it.
#   2. explicit directory + glob: directory walked, filtered by glob
#   3. no paths + glob: walks current directory filtered by glob
#   4. literal separator: '*.bin' matches only target dir, NOT subdir
#   5. '**/*.bin' matches recursively across subdirs
#   6. same glob rules hold for add, status, AND get
#   7. --recursive and --glob are mutually exclusive for status and get
#
# IMPORTANT: every glob passed to --glob/-g is QUOTED so the shell does not
# expand it; the spec requires lib-side expansion. One deliberate UNQUOTED case
# is included to contrast shell-expansion (add only, as specs.md, add, shows).

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

# Sorted, comma-joined list of the "path" of every row that succeeded (code
# null) in a --json envelope. A failed row (say get's not_tracked or
# hash_mismatch) drops out, so a check tests retrieval, not only selection.
json_paths() {
  { set +x; } 2>/dev/null
  python3 -c 'import sys,json
d=json.load(sys.stdin)
print(",".join(sorted(r["path"] for r in d["files"] if r.get("code") is None)))' 2>/dev/null \
    || echo "<parse-error>"
  { set -x; } 2>/dev/null
}

# "<code> hint" or "<code> no-hint" for the error of a failed --json envelope.
json_error() {
  { set +x; } 2>/dev/null
  python3 -c 'import sys,json
e=json.load(sys.stdin).get("error") or {}
print(e.get("code"), "hint" if e.get("hint") else "no-hint")' 2>/dev/null \
    || echo "<parse-error>"
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
dvs init local "$DVS_STORAGE"

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

# ═══════════════════════════════════════════════════════════════════════════
# ADD claims
# ═══════════════════════════════════════════════════════════════════════════

# ── Claim 4 (add): literal separator '*.bin' -> top-level only ──
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

# ── Claim 5 (add): '**/*.bin' recursive ──
say
say "=== ADD claim 5: --glob '**/*.bin' recursive ==="
out="$(dvs add --glob '**/*.bin' --json)"
check "$ALL_BIN" "$(printf '%s' "$out" | json_paths)" "add --glob '**/*.bin' matches recursively"
check "$ALL_BIN" "$(sidecar_paths)" "add '**/*.bin' wrote sidecars for all *.bin recursively"

reset_repo

# ── Claim 2 (add): explicit directory + glob -> dir walked, filtered ──
say
say "=== ADD claim 2: explicit dir 'data' + --glob '*.bin' ==="
out="$(dvs add data --glob '*.bin' --json)"
check "$DATA_DIR_BIN" "$(printf '%s' "$out" | json_paths)" "add data --glob '*.bin' walks data/ filtered by glob"

reset_repo

# ── Claim 1 (add): explicit files only + glob -> refused ──
# A glob only searches the directories among the paths, so it would select nothing.
say
say "=== ADD claim 1: explicit files keep1.txt keep2.txt + --glob '*.bin' (refused) ==="
rc=0
out="$(dvs add keep1.txt keep2.txt --glob '*.bin' --json)" || rc=$?
check "2" "$rc" "add with only files and --glob exits 2"
check "invalid_arguments no-hint" "$(printf '%s' "$out" | json_error)" "add with only files and a quoted --glob is invalid_arguments, with no quoting hint"
check "" "$(sidecar_paths)" "add with only files and --glob adds nothing"

reset_repo

# A file and a directory: the file as given, the glob searches the directory.
say
say "=== ADD claim 1b: keep1.txt + data + --glob '*.bin' ==="
out="$(dvs add keep1.txt data --glob '*.bin' --json)"
check "data/d1.bin,data/d2.bin,keep1.txt" "$(printf '%s' "$out" | json_paths)" "add with a file, a directory and --glob adds the file and the matches in the directory"

reset_repo

# Unquoted, the shell expands the glob: dvs sees the glob top1.bin and the file top2.bin.
say
say "=== ADD claim 1c: unquoted --glob *.bin (shell-expanded) is refused with a hint ==="
rc=0
# shellcheck disable=SC2035
out="$(dvs add --glob *.bin --json)" || rc=$?
check "2" "$rc" "add --glob *.bin (unquoted) exits 2"
check "invalid_arguments hint" "$(printf '%s' "$out" | json_error)" "add --glob *.bin (unquoted) is invalid_arguments, with a hint to quote the glob"
check "" "$(sidecar_paths)" "add --glob *.bin (unquoted) adds nothing"
# shellcheck disable=SC2035
err="$(dvs add --glob *.bin 2>&1 >/dev/null)" || true
# Key tokens, not wording: the glob dvs saw, and the same hint the --json
# error carries.
got="$(printf '%s' "$out" | python3 -c 'import sys,json
h=(json.load(sys.stdin).get("error") or {}).get("hint") or ""
e=sys.argv[1]
print("yes" if h and h in e and "top1.bin" in e else "no")' "$err" 2>/dev/null || echo "<parse-error>")"
check "yes" "$got" "the terminal error names the glob top1.bin and shows the --json hint"

reset_repo

# ── Claim 3 (add): no paths + glob -> walks cwd filtered ──
# (Same invocation form as claim 4/5; assert it walked the CURRENT dir.)
say
say "=== ADD claim 3: no paths, --glob '*.bin' walks current dir ==="
out="$(dvs add --glob '*.bin' --json)"
check "$TOP_ONLY" "$(printf '%s' "$out" | json_paths)" "add (no paths) --glob walks cwd filtered by glob"

reset_repo

# ── add bonus: UNQUOTED *.bin is expanded by the SHELL ──
say
say "=== ADD bonus: unquoted *.bin (shell expansion) ==="
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
# GET claims (get uses -g/--glob and resolves against the metadata folder)
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

# ── Claim 1 (get): explicit files only + glob -> refused ──
# The files are tracked and missing locally: a tracked file counts as a file.
say
say "=== GET claim 1: explicit files + --glob (refused) ==="
del_locals
rc=0
out="$(dvs get top1.bin top2.bin --glob '**/*.bin' --json)" || rc=$?
check "2" "$rc" "get with only files and --glob exits 2"
check "invalid_arguments no-hint" "$(printf '%s' "$out" | json_error)" "get with only files and --glob is invalid_arguments"
check "no" "$([ -e top1.bin ] && echo yes || echo no)" "get with only files and --glob retrieves nothing"

# A file and a directory: the file as given, the glob searches the directory.
say
say "=== GET claim 1b: top1.bin + data + --glob '*.bin' ==="
del_locals
out="$(dvs get top1.bin data --glob '*.bin' --json)"
check "data/d1.bin,data/d2.bin,top1.bin" "$(printf '%s' "$out" | json_paths)" "get with a file, a directory and --glob retrieves the file and the matches in the directory"

# ── Claim 2 (get): explicit dir + glob -> dir walked filtered (#217 fixed) ──
# Globbing: get resolves a glob the same way as add, and an explicit
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
# STATUS glob claims (specs.md, Globbing, lists status among the --glob commands)
# ═══════════════════════════════════════════════════════════════════════════
say
say "=== STATUS: accepts --glob and follows the literal-separator rule (#218) ==="
rc=0
status_out="$(dvs status --glob '*.bin' --json)" || rc=$?
say "status --glob exit code: $rc"
say "status --glob stdout: $status_out"
# Globbing: add, status, get all accept --glob. (#218, which had status reject
# the flag, is fixed.) The literal `*` does not cross directory separators, so
# '*.bin' matches only top-level .bin files.
check "0" "$rc" "status --glob is accepted (exit 0), not rejected (#218)"
got="$(printf '%s' "$status_out" | json_paths)"
check "$TOP_ONLY" "$got" "status --glob '*.bin' follows literal-separator rule (top-level only)"

# Explicit files only + glob: refused, as for add and get.
say
say "=== STATUS: explicit file + --glob is refused ==="
rc=0
out="$(dvs status top1.bin --glob '*.bin' --json)" || rc=$?
check "2" "$rc" "status with only files and --glob exits 2"
check "invalid_arguments no-hint" "$(printf '%s' "$out" | json_error)" "status with only files and --glob is invalid_arguments"

# -g is the short form of --glob.
say
say "=== STATUS: -g short flag is accepted ==="
rc=0
dvs status -g '*.bin' >/dev/null 2>&1 || rc=$?
check "0" "$rc" "status -g accepted (exit 0), short form of --glob (#218)"

# ═══════════════════════════════════════════════════════════════════════════
# --recursive and --glob are mutually exclusive: a usage error, exit 2
#, for both commands that take -r.
# ═══════════════════════════════════════════════════════════════════════════
say
say "=== STATUS/GET: -r with --glob is refused ==="
rc=0
dvs status -r data --glob '*.bin' >/dev/null 2>&1 || rc=$?
check "2" "$rc" "status -r --glob is a usage error, exit 2"
rc=0
dvs get -r data --glob '*.bin' >/dev/null 2>&1 || rc=$?
check "2" "$rc" "get -r --glob is a usage error, exit 2"

# ═══════════════════════════════════════════════════════════════════════════
say
say "=== SUMMARY: $PASS pass, $FAIL fail ==="
printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
