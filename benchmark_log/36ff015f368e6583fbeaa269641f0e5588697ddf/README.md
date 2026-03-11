# Benchmark: 36ff015 reverted (without "Fix save to not read" optimization)

## Code state

Branch: `vvv`
Built from commit `ec1ca91` with commit `36ff015 Fix save
to not read (#68)` reverted. This restores the pre-write `backend.read()`
call — before each storage write, the old content is read into memory
for rollback purposes.

## Machine

- Apple M3 Max, 36 GB RAM
- macOS Darwin 25.3.0 (arm64)
- dvs-cli 0.1.1

## Scripts

All 8 scripts ran sequentially (not in parallel). Each script uses
`mktemp` for project and storage directories. File content is fresh
random bytes per rep (`/dev/urandom`).

| File | Files per `dvs add` | Order | Compression |
|------|---------------------|-------|-------------|
| `bench_single_serial_results.csv` | 1 | serial | zstd |
| `bench_single_random_results.csv` | 1 | random | zstd |
| `bench_batch_serial_results.csv` | 5 | serial | zstd |
| `bench_batch_random_results.csv` | 5 | random | zstd |
| `bench_single_serial_no_compression_results.csv` | 1 | serial | none |
| `bench_single_random_no_compression_results.csv` | 1 | random | none |
| `bench_batch_serial_no_compression_results.csv` | 5 | serial | none |
| `bench_batch_random_no_compression_results.csv` | 5 | random | none |

## Parameters

- Sizes: 1, 2, 5, 10, 50 MB
- Reps: 25 per size
- Batch size: 5 files (batch scripts)
- Storage: local filesystem (tmpdir)

## CSV format

Columns `size_mb,rep` are prepended by the bash script. Remaining
columns come from the dvs `-vvv` timing CSV:

```
size_mb,rep,timestamp,user,dvs_version,git_commit,command,file,step,duration_ms,file_size_bytes,num_files,compression,hash_algorithm
```

Key `step` values: `hash`, `backend_store`, `add_file_total`, `add_total`.
