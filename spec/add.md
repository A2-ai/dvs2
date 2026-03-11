# `dvs add`

Adds files to DVS storage and records their metadata.

## Behavior

- Accepts files as input. Directories are not accepted directly but can be combined with `--glob` to match files within them.
- An optional message can be attached; it is recorded in the metadata file.
- Best-effort: if some files fail, the rest are still processed. The function never errors as a whole — individual failures are reported per-file in the result list.
- Results are sorted alphabetically by path.

### Outcomes

Each file reports one of:

- `copied`: the file was new or had changed; it was copied to storage and metadata was written.
- `present`: the file's hash and size match the existing metadata. No-op. The metadata file is not rewritten, so fields like `message` are not updated.

### Atomicity

Each file's add is atomic: the storage write and metadata update either both succeed or both roll back. A failure writing to storage will not leave behind a partial metadata file, and vice versa. Rollback restores any pre-existing metadata content.

### Symlinks

Symlinks are resolved before adding. If a symlink target resolves to a path outside the project root, the file is rejected.

### Dry run

`--dry-run` returns the outcome that would occur for each file without writing to storage or metadata.

### Path validation

Each file is checked before processing:

- `not found`: file does not exist on disk.
- `outside project`: path resolves to outside the project root.
- `is a directory`: path is a directory, not a file.

These are reported as per-file errors.

### Gitignore

After a successful add, each added file is appended to a `.gitignore` in the file's parent directory using the format `/<filename>`, unless it's already present. If the repository has no `.git` folder, gitignore updates are skipped. A failure to update `.gitignore` is logged as a warning but does not fail the operation.

### Audit trail

Every successfully added file is logged to the append-only `audit.log.jsonl` in the storage directory. Each entry records the operation ID (UUID grouping all files from one invocation), timestamp, username, file path, hashes, and action (`add`).

### Parallelism

Files are processed in parallel. Thread count is controlled by the `DVS_NUM_THREADS` environment variable, capped at 16 and clamped to the number of files.

### Hash cache

After a successful add, each file's hash is stored in a SQLite cache at `{metadata_folder}/.cache/dvs.db` keyed by `mtime` and `size`. Subsequent `status` checks benefit from cache hits.

## CLI

```
dvs add [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...              Files or directories to add

Options:
      --glob <GLOB>       Glob pattern to filter files
      --json              Output results as JSON
  -m, --message <MESSAGE> An optional message to record
      --dry-run           Show what would be added without making changes
  -v, --verbose           Show per-step timing information
  -h, --help              Print help
```

At least one path or `--glob` must be provided.

Shell expansion works: `dvs add *.csv` is expanded by the shell before calling dvs. Use `--glob` for consistent behavior with the R package.

### Globbing

- Explicit files: added directly (glob ignored).
- Explicit directories with a glob: directory is walked and filtered by the glob.
- No paths with a glob: current directory is walked and filtered by the glob.

Globs use a literal path separator: `*.csv` matches only in the target directory, not `subdir/file.csv`. Use `**/*.csv` for recursive matching.

### Output

Default:
```
Added: <path>
```

Errors are printed to stderr:
```
Error adding <path>: <reason>
```

JSON (`--json`):
```json
[
  {"path": "file.csv", "outcome": "copied", "hash": "<blake3>", "size": 12345},
  {"path": "missing.csv", "error": "file not found"}
]
```

### Exit codes

- `0`: all files added successfully.
- `1`: one or more files failed to add.

## Rust library

```rust
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
) -> Result<Vec<AddResult>>
```

The only error the function itself can return is failure to set up the thread pool. All per-file outcomes (success or error) are in the returned `Vec<AddResult>`.

```rust
pub struct AddResult {
    pub path: PathBuf,
    pub detail: AddDetail,
}

pub enum AddDetail {
    Success { outcome: Outcome, hash: String, size: u64 },
    Error { error: String },
}
```

## R package

```r
dvs_add(
  files,
  message = NULL
)
```

- `files`: character vector of file paths.
- `message`: optional string recorded in metadata.

Returns a data frame with columns: `path`, `outcome`, `hash`, `size`, `error`.

## Examples

### Add specific files

```bash
dvs add data.csv model.rds
```

### Add with a message

```bash
dvs add data.csv -m "Q4 refresh"
```

### Add files matching a glob

```bash
dvs add --glob "*.csv"
```

### Dry run

```bash
dvs add --dry-run data.csv
```
