# UI Test Scripts

## Running UI tests

```bash
bash ui/main.sh                  # init, add, status walkthrough (needs `just datasets`)
bash ui/main_init.sh             # dvs init local / dvs_init option matrix
bash ui/main_cli_help.sh         # --help option order, expected vs actual
bash ui/main_events.sh           # dvs_add / dvs_get / dvs_status result shapes
bash ui/main_partial_failure.sh  # Per-file add failure: CLI exit code, R error row
bash ui/main_status.sh           # Status filtering, path filtering, recursive
bash ui/main_recursive.sh        # Recursive flag across add/get/status
bash ui/main_progress.sh         # Progress bars: 100x1MB + 1x600MB
bash ui/main_parallel.sh         # Thread control benchmarks
bash ui/main_threads.sh          # Thread count precedence: flag, option, env
bash ui/main_log.sh              # Logging: off/info/debug/warn/error, threads, transitions
bash ui/cleanup.sh               # Remove temp dirs left by tests
```

`main.sh` builds CSV fixtures from `datasets/chickweight.csv`. Run `just datasets` once to download it. `datasets/` is gitignored.

All scripts use `set -euox pipefail` and trap on ERR, so a nonzero exit code means something failed. But **exit code 0 is not sufficient** — always redirect output to a log file, then read the log and verify:

1. CLI and R produce matching results (same row counts, same statuses, same file lists)
2. No R errors, warnings, or unexpected "Error in ..." messages buried in output
3. Dataframes have the expected columns and values for each scenario
4. Progress bars render. They draw only on a terminal, so a fully redirected log shows none. Run `main_progress.sh` with stderr on the terminal. The CLI draws a bar per file above 500 MiB. R draws `■` cli bars only in an interactive R session, so the Rscript calls in these scripts print none.

Put `> log 2>&1` in that order. `2>&1 > log` sends stderr to the terminal, and stderr carries the xtrace lines, the echoed R code, the dvs log lines and every error.

```bash
bash ui/main_status.sh > /tmp/ui-status.log 2>&1; echo "EXIT: $?"
# Then read /tmp/ui-status.log and verify output, don't just check exit code
```

```bash
bash ui/main_log.sh > /tmp/ui-log.log 2>&1; echo "EXIT: $?"
# Then read /tmp/ui-log.log and verify:
#   S1 — no log lines; S2/S3 — INFO/DEBUG dvs lines;
#   S4/S7 — single "Failed to add" warn line per scenario;
#   S5 — clean error message, no panic;
#   S6 — debug lines interleaved cleanly under 4 threads;
#   S8 — phase A debug, phase B silent, phase C one warn line.
```

## Cleanup

Tests create `dvs_repo_*`, `dvs_storage_*`, and `dvs_fixture_*` temp dirs under `ui/`. Run `bash ui/cleanup.sh` after each run. The cleanup script trashes (not deletes) these directories.
