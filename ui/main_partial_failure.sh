#!/usr/bin/env bash
#
# Showcase: per-file failure handling in add (issue #243).
#
# A batch with one readable file and one unreadable file (mode 000). The good
# file is added; the bad file is reported in the result's `error` column.
#
# The two front-ends signal the failure differently, on purpose. The CLI exits
# nonzero. The R binding does NOT raise an R warning or error for a per-file
# failure: dvs does not turn its own failures into R conditions. It returns the
# partial-result data frame and puts the failure in the `error` column, so the
# caller inspects the result rather than trapping a condition. This script
# asserts that the R call stays warning-free while the failure still shows up
# in the returned data frame.

set -eu
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"
set -xo pipefail

say "NOTE: \`just install-all\` should have been run first so the dvs CLI and the installed dvs R package both reflect this branch."

if [ "$(id -un)" = "root" ]; then
  say "Refusing to run as root: a mode-000 file is still readable by root, so the failure cannot be demonstrated."
  exit 1
fi

# region: SETUP — separate repos so the CLI and R runs do not see each other's files

DVS_REPO_CLI="$(mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_partfail_XXX)"
RUN_SUFFIX="${DVS_REPO_CLI##*_}"
DVS_STORAGE_CLI="$SCRIPT_DIR/dvs_storage_cli_partfail_$RUN_SUFFIX"
DVS_REPO_RPKG="$SCRIPT_DIR/dvs_repo_rpkg_partfail_$RUN_SUFFIX"
DVS_STORAGE_RPKG="$SCRIPT_DIR/dvs_storage_rpkg_partfail_$RUN_SUFFIX"
mkdir "$DVS_STORAGE_CLI" "$DVS_REPO_RPKG" "$DVS_STORAGE_RPKG"

# Restore perms on exit so cleanup.sh can remove the mode-000 files.
trap 'chmod 644 "$DVS_REPO_CLI"/locked.csv "$DVS_REPO_RPKG"/locked.csv 2>/dev/null || true' EXIT

seed_files() { # $1 = repo dir
  { set +x; } 2>/dev/null
  printf 'id,value\n1,alpha\n2,beta\n' > "$1/good.csv"
  printf 'id,value\n3,gamma\n4,delta\n' > "$1/locked.csv"
  chmod 000 "$1/locked.csv" # strip all perms so hashing fails with EACCES
  set -x
}

# region: CLI — nonzero exit, failure printed to stderr

cd "$DVS_REPO_CLI"
dvs init "$DVS_STORAGE_CLI"
seed_files "$DVS_REPO_CLI"

say "--- project before add (locked.csv is mode 000) ---"
tree -p --noreport "$DVS_REPO_CLI"

say "=== dvs CLI: add good.csv + locked.csv (expect nonzero exit) ==="
cli_exit=0
dvs add good.csv locked.csv || cli_exit=$?
say "CLI exit code: $cli_exit"
test "$cli_exit" -ne 0 # the CLI must signal the per-file failure with a nonzero exit

# region: R — no warning, failure in the partial-result data frame

cd "$DVS_REPO_RPKG"
seed_files "$DVS_REPO_RPKG"

print_eval_rscript <<EOF
library(dvs)

dvs_init("$DVS_STORAGE_RPKG")

message("=== R: dvs_add(c('good.csv', 'locked.csv')) ===")

# A per-file failure must be reported through the returned data frame only.
# Trip an error if any R warning is raised, so a silent regression to the old
# warning() behavior fails this walkthrough loudly.
result <- withCallingHandlers(
  dvs_add(c("good.csv", "locked.csv")),
  warning = function(w) stop("unexpected R warning: ", conditionMessage(w))
)
message("no R warning was raised: the per-file failure is data-frame-only")

print(result, width = Inf)

# locked.csv is reported in the \`error\` column with its \`outcome\` left NA,
# while good.csv is copied.
stopifnot(
  "result carries an error column" = "error" %in% names(result),
  "locked.csv has a populated error" =
    nzchar(result\$error[basename(result\$path) == "locked.csv"]),
  "good.csv was copied" =
    result\$outcome[basename(result\$path) == "good.csv"] == "copied"
)
EOF

say "--- R project after add: good.csv tracked (.dvs/good.csv.dvs), locked.csv not ---"
tree -a --noreport "$DVS_REPO_RPKG"

printf '\nCleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
