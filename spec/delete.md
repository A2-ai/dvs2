# `dvs delete`

**Status: not implemented.** This spec describes proposed behavior.

Provides a controlled way to delete files from DVS tracking and optionally from storage.

## Proposed behavior

- Deletes the metadata sidecar file (`.dvs` file).
- Optionally deletes the local data file.
- Optionally deletes the blob from backend storage.
- Removes the file's entry from `.gitignore`.
- Logs a deletion event to the audit trail.
- Best-effort: if some files fail, the rest are still processed.
- Providing no arguments does not delete all tracked files. Explicit paths are required.
- Files that are not tracked produce an error.

### `--cached` mode

With `--cached`, only the metadata file and storage blob are removed. The local data file is preserved. This is useful for untracking a file without deleting the working copy.

### Audit trail

A deletion event should be logged with `action: "delete"`. This requires extending the `Action` enum.

## Proposed CLI

```
dvs delete [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...              Files to delete from DVS

Options:
  -c, --cached            Keep local file, delete metadata and storage only
  -m, --message <MESSAGE> Optional message explaining the deletion
      --json              Output as JSON
  -h, --help              Print help
```

### Exit codes

- `0`: all files deleted successfully.
- `1`: one or more files failed.

## Proposed Rust library

```rust
pub fn delete_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    cached_only: bool,
    message: Option<String>,
) -> Result<Vec<DeleteResult>>
```

## Proposed R package

```r
dvs_delete(
  files,
  cached = TRUE,
  message = NULL
)
```

- `files`: character vector of file paths.
- `cached`: if `TRUE` (default), keep local file.
- `message`: optional deletion reason.
