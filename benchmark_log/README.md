# benchmark_log

Benchmark scripts for measuring `dvs add` performance across file sizes,
batch sizes, iteration orders, and compression settings.

## Quick start

Run all 8 benchmarks:

```bash
bash benchmark_log/run_all.sh <STORAGE_DIR> <PROJECT_DIR> [RESULT_DEST_DIR]
```

Results (CSVs + logs) are saved to `<RESULT_DEST_DIR>/<commit_hash>/`.
If `RESULT_DEST_DIR` is omitted, defaults to `benchmark_log/<commit_hash>/`.

### Environment variables

| Variable    | Default          | Description                              |
|-------------|------------------|------------------------------------------|
| `SIZES_MB`  | `1 2 5 10 50`   | Space-separated file sizes in MB         |
| `REPS`      | `25`             | Repetitions per size                     |
| `BATCH`     | `$REPS`          | Files per `dvs add` in batch scripts     |

Example — 5 reps of 100MB and 500MB files, 10 files per batch:

```bash
SIZES_MB="100 500" REPS=5 BATCH=10 bash benchmark_log/run_all.sh /tmp/stor /tmp/prj /tmp/results
```

## Scripts

### run_all.sh

Runs all 8 bench scripts sequentially. Between each run it deletes both
`STORAGE_DIR` and `PROJECT_DIR` to ensure a clean state. Each script's
stdout+stderr is saved to a `.log` file alongside the CSV results.

### bench_common.sh

Shared infrastructure sourced by all 8 bench scripts. Not executed directly.

#### Functions

**`bench_setup <STORAGE_DIR> <PROJECT_DIR> [--no-compression]`**

Sets up the benchmark environment:
- Parses `STORAGE_DIR` and `PROJECT_DIR` arguments.
- Reads `SIZES_MB`, `REPS`, `BATCH` from environment (with defaults).
- Resolves all paths to absolute.
- Creates directories, `cd`s into `PROJECT_DIR`.
- Runs `dvs init` (with `--no-compression` if passed).
- Sets globals: `STORAGE`, `PROJECT_DIR`, `SIZES_MB`, `REPS`, `BATCH`,
  `SCRIPT_NAME`, `CSV`, `HEADER_WRITTEN`, `BENCH_SCRIPT_DIR`.

**`generate_files <size_mb> <count>`**

Generates random binary files in the current directory using `/dev/urandom`.

- `count=1`: creates `f.bin`.
- `count>1`: creates `f1.bin`, `f2.bin`, ..., `f<count>.bin`.

Sets the `GENERATED_FILES` array with the created filenames.

Standalone usage:

```bash
source benchmark_log/bench_common.sh
generate_files 500 30    # creates f1.bin through f30.bin, each 500MB
echo "${GENERATED_FILES[@]}"
```

**`drain_timing <size_mb> <rep>`**

Reads the `dvs-timings-*.csv` file left by `dvs add -vvv` in `PROJECT_DIR`,
prepends `size_mb,rep` columns, appends to `$CSV`, and deletes the timing file.
Writes the CSV header on first call.

**`shuffle_work`**

Builds and shuffles all `(size, rep)` pairs into the `WORK` array.
Used by the `*_random*` scripts to randomize iteration order
(mitigates disk cache effects).

Each entry is `"<size>,<rep>"`. Iterate with:

```bash
for item in "${WORK[@]}"; do
    IFS=',' read -r size rep <<< "$item"
    # ...
done
```

**`bench_teardown`**

Copies the results CSV from `PROJECT_DIR` back to the script directory.

### Bench scripts (8 variants)

Each script sources `bench_common.sh`, calls `bench_setup`, runs its
specific loop, and calls `bench_teardown`. The 8 scripts are a matrix of:

| Axis        | Options              | Description                                  |
|-------------|----------------------|----------------------------------------------|
| File count  | single / batch       | 1 file vs `BATCH` files per `dvs add`        |
| Order       | serial / random      | Sizes in order vs shuffled `(size, rep)` pairs|
| Compression | (default) / no_compression | zstd vs none                          |

| Script                                  | Files per add | Order  | Compression |
|-----------------------------------------|---------------|--------|-------------|
| `bench_single_serial.sh`               | 1             | serial | zstd        |
| `bench_single_random.sh`               | 1             | random | zstd        |
| `bench_batch_serial.sh`                | BATCH         | serial | zstd        |
| `bench_batch_random.sh`                | BATCH         | random | zstd        |
| `bench_single_serial_no_compression.sh`| 1             | serial | none        |
| `bench_single_random_no_compression.sh`| 1             | random | none        |
| `bench_batch_serial_no_compression.sh` | BATCH         | serial | none        |
| `bench_batch_random_no_compression.sh` | BATCH         | random | none        |

Each script can also be run standalone:

```bash
SIZES_MB="10" REPS=5 bash benchmark_log/bench_single_serial.sh /tmp/stor /tmp/prj
```

### make_readme.sh

Generates a `README.md` template inside a results directory with machine info,
parameters, and CSV format documentation.

```bash
bash benchmark_log/make_readme.sh /path/to/results/<commit_hash>
```

## CSV format

The bench scripts prepend `size_mb,rep` columns. The remaining columns come
from the `dvs -vvv` timing CSV:

```
size_mb,rep,invocation_id,timestamp,user,dvs_version,git_commit,command,file,step,duration_ms,file_size_bytes,num_files,compression,hash_algorithm
```

- `invocation_id`: high-precision timestamp captured once at CLI entry.
  All rows from the same `dvs add` call share the same value.
- `step` values:
  - `hash` — time to hash one file (blake3).
  - `backend_store` — time to compress (if enabled) and write one file to storage.
  - `add_file_total` — total time for one file (hash + store + metadata + audit).
  - `add_total` — total time for the entire `dvs add` invocation. `num_files` is set on this row.

## Output structure

After `run_all.sh`, the results directory contains:

```
<RESULT_DEST_DIR>/<commit_hash>/
├── bench_single_serial_results.csv
├── bench_single_serial.log
├── bench_single_random_results.csv
├── bench_single_random.log
├── ...
└── bench_batch_random_no_compression.log
```
