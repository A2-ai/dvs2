#!/usr/bin/env bash

# Demonstrate that the thread-count override priority chain is wired
# correctly through dvs::set_num_threads.
#
# Priority (high → low):
#   1. set_num_threads(n)       (CLI: --threads N, R: options(dvs.num_threads=N))
#   2. DVS_NUM_THREADS env var
#   3. default (cpus * 4, capped at 16)
#
# We surface the resolved choice via the `dvs` crate's log feature:
#   log::debug!("thread pool: N threads (source: …, work_items=W)")
# env_logger is initialised by the CLI binary, so RUST_LOG=dvs=debug
# renders the line on stderr next to each `dvs add` invocation. The R
# package does not initialise env_logger, so its section demonstrates the
# chain through observable option/env state instead of log output.

set -euox pipefail
trap 'printf "ERROR at %s:%d\n" "${BASH_SOURCE[0]}" "$LINENO" >&2' ERR

echo "NOTE: \`just install-all\` should have been called prior to this so the dvs CLI binary on PATH and the installed dvs R package both reflect the current branch."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# shellcheck source=ui/helpers.sh
source "${SCRIPT_DIR}/helpers.sh"

export RUST_LOG="dvs=debug"

N_FILES="${1:-8}"
FILE_SIZE="${2:-1M}"

# ── Fixture (one set of files reused across scenarios) ──────────────

FIXTURES="$(mktemp -d "$SCRIPT_DIR"/dvs_fixture_XXX)"
RUN_SUFFIX="${FIXTURES##*_}"
cd "$FIXTURES"
mkfiles "$N_FILES" "$FILE_SIZE" data/derived

# Helper: each scenario gets a fresh repo+storage so we always exercise
# the add code path (no "already added" short-circuits).
fresh_repo() {
  local tag="$1"
  local repo="$SCRIPT_DIR/dvs_repo_cli_${tag}_${RUN_SUFFIX}"
  local storage="$SCRIPT_DIR/dvs_storage_cli_${tag}_${RUN_SUFFIX}"
  mkdir "$repo" "$storage"
  cp -r "$FIXTURES/data" "$repo/data"
  ( cd "$repo" && dvs init "$storage" >/dev/null )
  printf '%s\n' "$repo"
}

# ── CLI scenarios ───────────────────────────────────────────────────

say "=== [CLI 1] no flag, no env → expect source=default ==="
REPO="$(fresh_repo s1)"
( cd "$REPO" && unset DVS_NUM_THREADS && dvs add data/derived/file_*.bin >/dev/null )

say "=== [CLI 2] DVS_NUM_THREADS=3, no flag → expect source=environment, threads=3 ==="
REPO="$(fresh_repo s2)"
( cd "$REPO" && DVS_NUM_THREADS=3 dvs add data/derived/file_*.bin >/dev/null )

say "=== [CLI 3 PRECEDENCE] --threads 7 beats DVS_NUM_THREADS=3 → override wins, threads=7 ==="
REPO="$(fresh_repo s3)"
( cd "$REPO" && DVS_NUM_THREADS=3 dvs --threads 7 add data/derived/file_*.bin >/dev/null )

say "=== [CLI 4 PRECEDENCE-CLEAR] --threads 0 clears override → env=3 takes over ==="
REPO="$(fresh_repo s4)"
( cd "$REPO" && DVS_NUM_THREADS=3 dvs --threads 0 add data/derived/file_*.bin >/dev/null )

# ── R scenarios ─────────────────────────────────────────────────────
# Each phase drives a real dvs_add → dvs_status → dvs_get round trip so
# dvs_set_threads_impl(getOption("dvs.num_threads")) fires before every op.
# That bridge is the only thing the R package adds on top of the core
# priority logic, so each phase exercises the full chain.
#
# Note: the R package does not initialise env_logger, so the
# dvs::utils::get_threadpool debug line is NOT rendered from R. Verification
# from R is by observable option/env state at each call site; the log-line
# verification of the priority logic itself comes from the CLI section above
# (same dvs::utils::get_threadpool code path).

# Each R phase needs its own fresh repo so dvs_add isn't a no-op.
DVS_REPO_RA="$SCRIPT_DIR/dvs_repo_rpkg_RA_$RUN_SUFFIX"
DVS_STORAGE_RA="$SCRIPT_DIR/dvs_storage_rpkg_RA_$RUN_SUFFIX"
DVS_REPO_RB="$SCRIPT_DIR/dvs_repo_rpkg_RB_$RUN_SUFFIX"
DVS_STORAGE_RB="$SCRIPT_DIR/dvs_storage_rpkg_RB_$RUN_SUFFIX"
DVS_REPO_RC="$SCRIPT_DIR/dvs_repo_rpkg_RC_$RUN_SUFFIX"
DVS_STORAGE_RC="$SCRIPT_DIR/dvs_storage_rpkg_RC_$RUN_SUFFIX"
DVS_REPO_RD="$SCRIPT_DIR/dvs_repo_rpkg_RD_$RUN_SUFFIX"
DVS_STORAGE_RD="$SCRIPT_DIR/dvs_storage_rpkg_RD_$RUN_SUFFIX"
DVS_REPO_RE="$SCRIPT_DIR/dvs_repo_rpkg_RE_$RUN_SUFFIX"
DVS_STORAGE_RE="$SCRIPT_DIR/dvs_storage_rpkg_RE_$RUN_SUFFIX"
for d in "$DVS_REPO_RA" "$DVS_STORAGE_RA" \
         "$DVS_REPO_RB" "$DVS_STORAGE_RB" \
         "$DVS_REPO_RC" "$DVS_STORAGE_RC" \
         "$DVS_REPO_RD" "$DVS_STORAGE_RD" \
         "$DVS_REPO_RE" "$DVS_STORAGE_RE"; do
  mkdir "$d"
done
for d in "$DVS_REPO_RA" "$DVS_REPO_RB" "$DVS_REPO_RC" "$DVS_REPO_RD" "$DVS_REPO_RE"; do
  cp -r "$FIXTURES/data" "$d/data"
done

say "=== [R] priority chain across multiple ops ==="
print_eval_rscript <<EOF
library(dvs)

# Opt in to DVS internals log routing so the
#   "thread pool: N threads (source: …, work_items=W)"
# line from dvs::utils renders in R's console — same line the CLI section
# above prints, proving the priority chain resolves identically.
set_dvs_log_level("debug")

show <- function(label) {
  opt <- getOption("dvs.num_threads")
  env <- Sys.getenv("DVS_NUM_THREADS", unset = NA)
  cat(sprintf(
    "  state> %-40s option=%-6s env=%s\n",
    label,
    if (is.null(opt)) "NULL" else as.character(opt),
    if (is.na(env)) "<unset>" else env
  ))
}

run_phase <- function(label, repo, storage) {
  cat(sprintf("\n--- %s ---\n", label))
  show("entering phase")
  setwd(repo)
  dvs_init(storage)
  add_df    <- dvs_add(glob = "data/derived/*")
  status_df <- dvs_status()
  # Remove a tracked file so dvs_get has actual work to do.
  unlink(file.path(repo, "data/derived/file_1.bin"))
  get_df    <- dvs_get(glob = "data/derived/*")
  cat(sprintf(
    "  ops>   add=%d rows  status=%d rows  get=%d rows\n",
    nrow(add_df), nrow(status_df), nrow(get_df)
  ))
}

# ── Phase A: option NULL, env unset → expect default ──
Sys.unsetenv("DVS_NUM_THREADS")
set_dvs_threads(NULL)
run_phase("Phase A: option=NULL, env=unset (expect default)",
          "$DVS_REPO_RA", "$DVS_STORAGE_RA")

# ── Phase B: env=3, option NULL → expect env=3 ──
Sys.setenv(DVS_NUM_THREADS = "3")
set_dvs_threads(NULL)
run_phase("Phase B: option=NULL, env=3 (expect env=3)",
          "$DVS_REPO_RB", "$DVS_STORAGE_RB")

# ── Phase C PRECEDENCE: env=3, option=7 → set_dvs_threads beats env ──
Sys.setenv(DVS_NUM_THREADS = "3")
set_dvs_threads(7L)
run_phase("Phase C PRECEDENCE: set_dvs_threads(7) beats env=3 (expect override=7)",
          "$DVS_REPO_RC", "$DVS_STORAGE_RC")

# ── Phase D: withr scope (option=5 inside) ──
Sys.setenv(DVS_NUM_THREADS = "3")
set_dvs_threads(NULL)
withr::with_options(list(dvs.num_threads = 5L), {
  run_phase("Phase D: withr option=5, env=3 (expect override=5 inside)",
            "$DVS_REPO_RD", "$DVS_STORAGE_RD")
})
cat("\n--- after withr block ---\n")
show("scope reverted")

# ── Phase E PRECEDENCE TOGGLE ──
# Hold DVS_NUM_THREADS=3 constant; toggle set_dvs_threads() between ops.
# Expected log sequence within this single phase:
#   op1 (option=NULL) → source=environment,      threads=3
#   op2 (option=7L)   → source=override, threads=7   <- override wins over env
#   op3 (option=5L)   → source=override, threads=5   <- still override, different value
#   op4 (option=NULL) → source=environment,      threads=3   <- clearing override falls back to env
Sys.setenv(DVS_NUM_THREADS = "3")
cat("\n--- Phase E PRECEDENCE TOGGLE: env=3 held; toggle set_dvs_threads ---\n")
setwd("$DVS_REPO_RE")
dvs_init("$DVS_STORAGE_RE")

set_dvs_threads(NULL)
show("op1 before dvs_add (option=NULL → expect env=3)")
dvs_add(glob = "data/derived/*")

set_dvs_threads(7L)
show("op2 before dvs_status (option=7 → expect override=7)")
dvs_status()

set_dvs_threads(5L)
show("op3 before dvs_get (option=5 → expect override=5)")
unlink(file.path("$DVS_REPO_RE", "data/derived/file_1.bin"))
dvs_get(glob = "data/derived/*")

set_dvs_threads(NULL)
show("op4 before dvs_status (option=NULL → expect env=3 again)")
dvs_status()

# Final cleanup of process state.
Sys.unsetenv("DVS_NUM_THREADS")
set_dvs_threads(NULL)
EOF

# ── Summary ─────────────────────────────────────────────────────────

say ""
say "=== Summary ==="
say "CLI scenarios above print 'thread pool: N threads (source: …)' to stderr."
say "Look for:"
say "  [CLI 1]            source=default"
say "  [CLI 2]            source=environment,      threads=3"
say "  [CLI 3 PRECEDENCE] source=override, threads=7   (--threads 7 beats env=3)"
say "  [CLI 4 CLEAR]      source=environment,      threads=3   (--threads 0 clears, env wins)"
say ""
say "R phases drive add → status → get under different option/env"
say "configurations. The same dvs::utils::get_threadpool() priority chain"
say "applies, with set_dvs_log_level('debug') routing the log line to R's"
say "console. Look for:"
say "  Phase A             3× source=default,  threads=8 (option=NULL, env unset)"
say "  Phase B             3× source=environment,      threads=3 (option=NULL, env=3)"
say "  Phase C PRECEDENCE  3× source=override, threads=7 (option=7 beats env=3)"
say "  Phase D PRECEDENCE  3× source=override, threads=5 (withr option=5 beats env=3)"
say "  Phase E TOGGLE      env=3 held; option toggled NULL→7→5→NULL between ops:"
say "                      op1 source=environment      threads=3"
say "                      op2 source=override threads=7   <- option wins over env"
say "                      op3 source=override threads=5   <- still wins, different N"
say "                      op4 source=environment      threads=3   <- clear → env returns"
say ""
printf 'Cleanup: bash %s/cleanup.sh\n' "$SCRIPT_DIR"
