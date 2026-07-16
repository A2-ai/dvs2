# UI Test Scripts

## Running UI tests

```bash
bash ui/main_status.sh       # Status filtering, path filtering, recursive
bash ui/main_recursive.sh    # Recursive flag across add/get/status
bash ui/main_progress.sh     # Progress bars: 100x1MB + 1x500MB
bash ui/main_parallel.sh     # Thread control benchmarks
bash ui/main_log.sh          # Logging: off/info/debug/warn/error, threads, transitions
bash ui/cleanup.sh           # Remove temp dirs left by tests
```

All scripts use `set -euox pipefail` and trap on ERR, so a nonzero exit code means something failed. But **exit code 0 is not sufficient** — always redirect output to a log file, then read the log and verify:

1. CLI and R produce matching results (same row counts, same statuses, same file lists)
2. No R errors, warnings, or unexpected "Error in ..." messages buried in output
3. Dataframes have the expected columns and values for each scenario
4. Progress bars render (look for the `■` characters in xtrace output)

```bash
bash ui/main_status.sh 2>&1 > /tmp/ui-status.log; echo "EXIT: $?"
# Then read /tmp/ui-status.log and verify output, don't just check exit code
```

```bash
bash ui/main_log.sh 2>&1 > /tmp/ui-log.log; echo "EXIT: $?"
# Then read /tmp/ui-log.log and verify:
#   S1 — no log lines; S2/S3 — INFO/DEBUG dvs lines;
#   S4/S7 — single "Failed to add" warn line per scenario;
#   S5 — clean error message, no panic;
#   S6 — debug lines interleaved cleanly under 4 threads;
#   S8 — phase A debug, phase B silent, phase C one warn line.
```

## Cleanup

Tests create `dvs_repo_*`, `dvs_storage_*`, and `dvs_fixture_*` temp dirs under `ui/`. Run `bash ui/cleanup.sh` after each run. The cleanup script trashes (not deletes) these directories.

## Demonstrating fixes (evidence for PRs)

A PR that claims to fix broken behavior must show the breakage actually existed and is gone. The procedure:

1. Install the CLI from origin/main: `just install-cli` from a worktree checked out at origin/main. Run the failing scenario with that installed `dvs` and capture the transcript verbatim (commands, errors, exit codes, resulting `.dvs` sidecars).
2. Add a `ui/main_*.sh` walkthrough on the fix branch that sets up the same cases and asserts the FIXED behavior. It doubles as the regression walkthrough after merge.
3. Run the walkthrough twice: once with the branch build first on PATH (`PATH="$PWD/target/debug:$PATH"`) to show it passes, once against the installed origin/main `dvs` to show it fails at the broken case.
4. Post both transcripts in the PR as a collapsible `<details>` block that a reader can follow top to bottom: setup, broken output on main, fixed output on the branch, and where the script lives.
5. Leave the environment on main afterwards: reinstall the CLI (and the R package if it was replaced) from origin/main.

If step 1 shows the scenario is NOT broken on main, the fix may be redundant. Say so on the PR instead of pushing the script (see #266 for a case where the resolver already covered it).
