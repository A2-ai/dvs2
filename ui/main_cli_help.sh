#!/usr/bin/env bash

# CLI help-output ordering test.
#
# Goal: verify that `dvs <subcommand> --help` lists options in the order they
# are declared in dvs-cli/src/main.rs. The three `Cli`-level globals --json,
# --emit-events and --threads carry `help_heading = "Global Options"`, so
# clap prints them in their own section after each subcommand's local
# Options. `init` is a group: its options live on `init local` and
# `init server`.
#
# This script prints, for each subcommand:
#   1. The expected order: arguments, then local options (in declaration
#      order from main.rs), then the Global Options section.
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
echo "  Commands:"
echo "    init"
echo "    attach"
echo "    add"
echo "    status"
echo "    get"
echo "    info"
echo "    availability"
echo "    history"
echo "    login"
echo "    logout"
echo "    help"
echo "  Options:"
echo "    -h, --help"
echo "    -V, --version"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

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
echo "--- expected: the two backends, then Global Options ---"
echo "  Commands:"
echo "    local"
echo "    server"
echo "    help"
echo "  Options:"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

echo
echo "--- actual ---"
dvs init --help

# ============================================================================
# dvs init local --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs init local --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: locals, then Global Options ---"
echo "  <PATH>"
echo "  Options:"
echo "    --root-dir"
echo "    --metadata-folder-name"
echo "    --group"
echo "    --no-compression"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

echo
echo "--- actual ---"
dvs init local --help

# ============================================================================
# dvs init server --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs init server --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: locals, then Global Options ---"
echo "  <NAME>"
echo "  <URL>"
echo "  Options:"
echo "    --metadata-folder-name"
echo "    --group"
echo "    --no-compression"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

echo
echo "--- actual ---"
dvs init server --help

# ============================================================================
# dvs add --help
# ============================================================================

echo
echo "════════════════════════════════════════════════════════════"
echo "  dvs add --help"
echo "════════════════════════════════════════════════════════════"

echo
echo "--- expected: locals, then Global Options ---"
echo "  [PATHS]..."
echo "  Options:"
echo "    --glob"
echo "    -m, --message"
echo "    --dry-run"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

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
echo "--- expected: locals, then Global Options ---"
echo "  [PATHS]..."
echo "  Options:"
echo "    -r, --recursive"
echo "    -g, --glob"
echo "    --current"
echo "    --absent"
echo "    --stale"
echo "    --modified"
echo "    --unsynced"
echo "    --with-metadata"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

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
echo "--- expected: locals, then Global Options ---"
echo "  [PATHS]..."
echo "  Options:"
echo "    -g, --glob"
echo "    -r, --recursive"
echo "    --dry-run"
echo "    --on-conflict"
echo "    -f, --force"
echo "    -h, --help"
echo "  Global Options:"
echo "    --json"
echo "    --emit-events"
echo "    --threads"

echo
echo "--- actual ---"
dvs get --help

echo
echo "Done. Compare the \"expected\" and \"actual\" blocks above for each subcommand."
