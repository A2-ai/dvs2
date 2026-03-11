# `dvs get`

Retrieves files from DVS storage into the project.

## Behavior

- Resolves which files to retrieve by scanning the metadata folder for `.dvs` sidecar files.
- Accepts files, directories, or globs. Paths and globs are resolved against the metadata folder, not the working tree.
- Best-effort: if some files fail, the rest are still processed. The function never errors as a whole — individual failures are reported per-file in the result list.
- Results are sorted alphabetically by path.

### Outcomes

Each file reports one of:

- `copied`: the file was retrieved from storage.
- `present`: the local file already matches the metadata (hash and size). No-op.

### Hash verification

After retrieval, the file's blake3 hash is verified against the metadata. If the hash does not match, the retrieved file is deleted and the operation fails for that file.

### Dry run

`--dry-run` returns the outcome that would occur for each file without writing any files to disk.

### Path validation

Each file is checked before processing:

- `not found`: no metadata file and no file on disk.
- `not tracked by DVS`: file exists on disk but has no metadata (not tracked).

These are reported as per-file errors.

### Parallelism

Files are processed in parallel. Thread count is controlled by the `DVS_NUM_THREADS` environment variable, capped at 16 and clamped to the number of files.

### Hash cache

After a successful retrieval, each file's hash is stored in the SQLite cache. Subsequent `status` checks benefit from cache hits.

## CLI

```
dvs get [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...          Files or directories to retrieve

Options:
  -g, --glob <GLOB>   Glob pattern to filter tracked files
      --json          Output results as JSON
      --dry-run       Show what would be retrieved without making changes
  -h, --help          Print help
```

At least one path or `--glob` must be provided.

### Globbing

- Explicit files: retrieved directly.
- Explicit directories: all tracked files under the directory are retrieved.
- Glob with no paths: all tracked files under current directory matching the glob are retrieved.

Globs use a literal path separator: `*.csv` matches only in the target directory. Use `**/*.csv` for recursive matching.

### Output

Default (only copied files are printed):
```
<path> [<size>]
Total: <n> files, <total size>
```

Errors are printed to stderr:
```
Error: <path> - <reason>
```

JSON (`--json`):
```json
[
  {"path": "file.csv", "outcome": "copied", "size": 12345},
  {"path": "missing.csv", "error": "file not found"}
]
```

### Exit codes

- `0`: all files retrieved successfully.
- `1`: one or more files failed to retrieve.

## Rust library

```rust
pub fn get_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    dry_run: bool,
) -> Result<Vec<GetResult>>
```

The only error the function itself can return is failure to set up the thread pool. All per-file outcomes (success or error) are in the returned `Vec<GetResult>`.

```rust
pub struct GetResult {
    pub path: PathBuf,
    pub detail: GetDetail,
}

pub enum GetDetail {
    Success { outcome: Outcome, size: u64 },
    Error { error: String },
}
```

## R package

```r
dvs_get(files)
```

- `files`: character vector of file paths to retrieve.

Returns a data frame with columns: `path`, `outcome`, `size`, `error`.

## Examples

### Retrieve specific files

```bash
dvs get data.csv model.rds
```

### Retrieve all tracked files in a directory

```bash
dvs get data/
```

### Retrieve files matching a glob

```bash
dvs get --glob "**/*.csv"
```

### Dry run

```bash
dvs get --dry-run data.csv
```
