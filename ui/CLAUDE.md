# UI Test Scripts

## Running UI tests

```bash
bash ui/main_status.sh    # Status filtering, path filtering, recursive
bash ui/main_progress.sh  # Progress bars: 100x1MB + 1x500MB
bash ui/main_parallel.sh  # Thread control benchmarks
bash ui/cleanup.sh        # Remove temp dirs left by tests
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

## Cleanup

Tests create `dvs_repo_*`, `dvs_storage_*`, and `dvs_fixture_*` temp dirs under `ui/`. Run `bash ui/cleanup.sh` after each run. The cleanup script trashes (not deletes) these directories.
