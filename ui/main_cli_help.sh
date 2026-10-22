#!/usr/bin/env bash

# CLI help-output ordering test.
#
# Goal: verify that `dvs <subcommand> --help` lists options in the order they
# are declared in dvs-cli/src/main.rs.  The two `Cli`-level globals --json
# and --threads carry display_order = 0 / 1, and each Command variant sets
# next_display_order = 100 — so the globals always render at the top of
# every subcommand's Options section, ahead of the locals.
#
# This script prints, for each subcommand:
#   1. The expected option order — Cli globals first, then locals (in
#      declaration order from main.rs).
#   2. The actual `--help` output produced by the installed `dvs` binary.
# The reader compares the two visually.
#
# Unlike the other ui/main_*.sh scripts this one runs without `set -x`:
# it only prints help text, so xtrace adds noise without value, and the
# `say` helper from helpers.sh would re-enable xtrace at the end of every
# call.

set -euo pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

echo "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH reflects the current branch."

# ============================================================================
# dvs --help — subcommand order
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs --help — subcommand order"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected (main.rs Command enum, top to bottom; clap appends 'help') ---"
echo "  init"
echo "  add"
echo "  status"
echo "  get"
echo "  help"

echo
echo "--- actual ---"
dvs --help

# ============================================================================
# dvs init --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs init --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: Cli globals first, then locals ---"
echo "  <PATH>"
echo "  --json"
echo "  --threads"
echo "  --root-dir"
echo "  --metadata-folder-name"
echo "  --group"
echo "  --no-compression"

echo
echo "--- actual ---"
dvs init --help

# ============================================================================
# dvs add --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs add --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: Cli globals first, then locals ---"
echo "  [PATHS]"
echo "  --json"
echo "  --threads"
echo "  --glob"
echo "  --message"
echo "  --dry-run"

echo
echo "--- actual ---"
dvs add --help

# ============================================================================
# dvs status --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs status --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: Cli globals first, then locals ---"
echo "  [PATHS]"
echo "  --json"
echo "  --threads"
echo "  --recursive"
echo "  --current"
echo "  --absent"
echo "  --unsynced"
echo "  --with-metadata"

echo
echo "--- actual ---"
dvs status --help

# ============================================================================
# dvs get --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs get --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: Cli globals first, then locals ---"
echo "  [PATHS]"
echo "  --json"
echo "  --threads"
echo "  --glob"
echo "  --dry-run"

echo
echo "--- actual ---"
dvs get --help

echo
echo "Done. Compare the \"expected\" and \"actual\" blocks above for each subcommand."
