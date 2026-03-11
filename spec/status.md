# `dvs status`

Reports the status of all tracked files in the project.

## Behavior

- Scans all `.dvs` sidecar files in the metadata folder to discover tracked files.
- For each tracked file, compares the local file against stored metadata.
- Does not accept paths or globs. It always reports on all tracked files in the project.
- Best-effort: individual files that cannot be inspected (e.g., permission errors) are reported as per-file errors. The function never errors as a whole.
- Results are sorted alphabetically by path.

### Statuses

Each tracked file reports one of:

- `current`: local file exists and its hash and size match the metadata.
- `absent`: metadata exists but the local file is missing.
- `unsynced`: local file exists but its hash or size differs from the metadata.

Note: `untracked` is an internal status used when checking individual files. `dvs status` only reports on tracked files (those with metadata), so `untracked` does not appear in its output.

### Filter flags

By default (no flags), all tracked files are shown regardless of state. The `--current`, `--absent`, and `--unsynced` flags are filters: when one or more are provided, only files matching those states are shown. Multiple flags can be combined.

### Parallelism

File status checks run in parallel. Thread count is controlled by the `DVS_NUM_THREADS` environment variable, capped at 16 and clamped to the number of files.

### Hash cache

Status checks use the SQLite hash cache. Files whose `mtime` and `size` match a cache entry skip re-hashing.

## CLI

```
dvs status [OPTIONS]

Options:
      --current   Include files that are current
      --json      Output results as JSON
      --absent    Include files that are absent
      --unsynced  Include files that are unsynced
  -h, --help      Print help
```

### Output

Default:
```
<path>: <Status>
```

When no tracked files exist:
```
No tracked files
```

When filters are active but no files match:
```
No tracked files matching the filter
```

Errors are printed to stderr:
```
Error getting status for <path>: <reason>
```

JSON (`--json`):
```json
[
  {"path": "data.csv", "status": "current"},
  {"path": "old.csv", "status": "absent"},
  {"path": "model.rds", "error": "permission denied"}
]
```

### Exit codes

- `0`: all statuses retrieved successfully.
- `1`: one or more files failed to get status.

## Rust library

```rust
pub fn get_status(paths: &DvsPaths) -> Result<Vec<FileStatus>>
```

The only error the function itself can return is failure to set up the thread pool. All per-file outcomes are in the returned `Vec<FileStatus>`.

```rust
pub struct FileStatus {
    pub path: PathBuf,
    pub detail: StatusDetail,
}

pub enum StatusDetail {
    Success { status: Status },
    Error { error: String },
}

pub enum Status {
    Untracked,
    Current,
    Absent,
    Unsynced,
}
```

## R package

```r
dvs_status()
```

Takes no arguments. Returns a data frame with columns: `path`, `status`, `error`.

## Examples

### Show all tracked files

```bash
dvs status
```

### Show only absent files

```bash
dvs status --absent
```

### Show absent and unsynced files

```bash
dvs status --absent --unsynced
```

### JSON output

```bash
dvs status --json
```
