Done. Three files in ui/scripts/bench/:

  generate_data.R — Calibrates bytes-per-row at runtime, then
  generates:

- Single files: 1, 5, 10, 50, 100, 500, 1000, 10000 MB
- Parallel sets: 20 files each at 1, 5, 10, 50, 100 MB
- Uses data.table::fwrite if available, falls back to
  write.table
- Writes in 500k-row chunks to keep memory manageable for
  the 10GB file

  bench.R — Takes a mode argument (rpkg or cli):

- rpkg: uses library(dvs) → dvs_init(), dvs_add(),
  dvs_status()
- cli: uses system2("dvs", ...) → dvs init, dvs add, dvs
  status
- Creates separate project+storage dirs per file size (so
  they don't interfere)
- Uses hardlinks from the generated data (no extra disk
  space)
- Add benchmark: times each single file add, then batch add
  of 20 files per size
- Status benchmark: 5 replicates cycling through all sizes
  before repeating (avoids cache effects)
- Saves raw CSV + prints summary with mean ± sd

  run.sh — Orchestrates everything:
  bash ui/scripts/bench/run.sh [base_dir] [storage_base]

# defaults: /tmp/dvs_bench_<timestamp>  /data/dvs/dvs_bench

  Runs generate → rpkg benchmark → cli benchmark →
  head-to-head comparison table (ratio of cli/rpkg times).

  Outputs: results_rpkg.csv, results_cli.csv,
  results_combined.csv with columns: tool, operation,
  test_type, size_mb, n_files, replicate, elapsed_sec.
