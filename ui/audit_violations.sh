#!/usr/bin/env bash
#
# Regression audit for the CLI spec violations found by the validate_*.sh sweep.
# Re-run this later to check whether each known divergence still holds.
#
# Each case asserts the SPEC-CORRECT behavior.
#   Section A — clear CLI bugs (#216, #217, #219): FATAL. Flip to PASS once the
#               CLI is fixed; the script's exit code = number of Section A failures.
#   Section B — spec/CLI divergences (#218, #220): INFORMATIONAL. These are
#               likely resolved by editing specs.md, which this script cannot
#               observe — update the case (or delete it) once the spec is settled.
#
# The dvs CLI must already be on PATH (`just install-cli`). CLI only; no R.
# Temp repos/storage land under ui/ (dvs_repo_cli_*/dvs_storage_cli_*) and are
# trashed by ui/cleanup.sh at the end.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail_a=0

note()    { printf '\n=== %s ===\n' "$*"; }
verdict() { # verdict <0=ok|1=bad> <label>
  if [ "$1" -eq 0 ]; then echo "PASS: $2"; else echo "FAIL: $2"; fail_a=$((fail_a + 1)); fi
}

# Fresh, pristine project per case (avoids cross-test storage contamination,
# itself a symptom noted in #216). Sets REPO and STORAGE, leaves cwd in REPO.
fresh_project() {
  REPO="$(mktemp -d "$SCRIPT_DIR/dvs_repo_cli_XXX")"
  STORAGE="$SCRIPT_DIR/dvs_storage_cli_${REPO##*_}"   # sibling, outside the repo
  mkdir -p "$STORAGE"
  ( cd "$REPO" && git init -q )
  cd "$REPO"
  dvs init "$STORAGE" >/dev/null
}

echo "######## Section A: CLI regression guards (fatal) ########"

# ── #216 — adding identical-content files in one invocation (default threads) ──
note "#216 add two byte-identical files in a single invocation"
fresh_project
head -c 1048576 /dev/urandom > a.bin
cp a.bin b.bin
rc=0; dvs add a.bin b.bin --json || rc=$?
ok=1
[ "$rc" -eq 0 ] && [ -f .dvs/a.bin.dvs ] && [ -f .dvs/b.bin.dvs ] && ok=0
verdict "$ok" "#216 both identical files added (exit0 + both sidecars; should dedup to one blob)"

# ── #217 — get with a positional directory + glob ──
note "#217 get <dir> --glob (positional directory argument)"
fresh_project
mkdir -p data
head -c 1024 /dev/urandom > data/d1.bin
head -c 1024 /dev/urandom > data/d2.bin
dvs add --glob 'data/*.bin' >/dev/null 2>&1
rm -f data/d1.bin data/d2.bin
rc=0; dvs get data --glob '*.bin' --json || rc=$?
ok=1
[ "$rc" -eq 0 ] && [ -f data/d1.bin ] && [ -f data/d2.bin ] && ok=0
verdict "$ok" "#217 'get data --glob \"*.bin\"' retrieves the files (like 'add')"

# ── #219/#240 — add valid + unresolvable (missing) path refuses the batch ──
note "#240 add valid + missing path refuses the whole batch (all-or-nothing, specs.md L146/L186)"
fresh_project
head -c 1024 /dev/urandom > ok.bin
rc=0; out="$(dvs add ok.bin missing.bin --json 2>&1)" || rc=$?
ok=1
[ ! -e .dvs/ok.bin.dvs ] \
  && printf '%s' "$out" | grep -q 'missing.bin' \
  && [ "$rc" -eq 1 ] && ok=0
verdict "$ok" "#240 all-or-nothing: ok.bin NOT added, missing.bin named, exit 1"

echo
echo "######## Section B: spec divergences (informational) ########"

# ── #218 — spec says status accepts --glob; CLI rejects it ──
note "#218 status --glob"
fresh_project
out="$(dvs status --glob '*.bin' 2>&1)" || true
if printf '%s' "$out" | grep -q 'unexpected argument'; then
  echo "DIVERGES: #218 'status' rejects --glob, but specs.md L431 lists it (likely fix: edit spec)"
else
  echo "ALIGNED:  #218 'status' accepts --glob"
fi

# ── #220/#240 — get refuses the batch on an explicitly-requested unknown path ──
note "#240 get valid + unknown path refuses the batch"
fresh_project
head -c 1024 /dev/urandom > f.bin
dvs add f.bin >/dev/null 2>&1
rm -f f.bin
rc=0; dvs get f.bin nope.bin --json >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 1 ] && [ ! -e f.bin ]; then
  echo "ALIGNED:  #240 get exits 1 and retrieves nothing when an explicit path is unknown"
else
  echo "DIVERGES: #240 get exit=$rc, f.bin present=$([ -e f.bin ] && echo yes || echo no); unknown path no longer refuses the batch"
fi

cd "$SCRIPT_DIR"
bash "$SCRIPT_DIR/cleanup.sh" >/dev/null 2>&1 || true

printf '\n=== Section A (CLI regression guards): %d fail ===\n' "$fail_a"
echo "Section B items are spec questions; update/remove their cases here once specs.md is settled."
exit "$fail_a"
