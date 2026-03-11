# Configuration

DVS configuration lives in `dvs.toml` at the project root. Created by `dvs init`.

## `dvs.toml` format

```toml
compression = "zstd"

[backend]
path = "/absolute/path/to/storage"
```

With optional group:

```toml
compression = "zstd"

[backend]
path = "/absolute/path/to/storage"
group = "groupname"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `compression` | `"zstd"` or `"none"` | `"zstd"` | Compression algorithm for stored files |
| `metadata_folder_name` | string | `.dvs` | Name of the metadata folder at project root. Omitted from TOML when using default. |
| `backend.path` | string (absolute path) | required | Storage directory path |
| `backend.group` | string | omitted | Unix group for storage directory and files |

### Project discovery

DVS finds the project root by walking up from the current directory until it finds a `dvs.toml`. If none is found, the current directory is used as the root.

## Metadata folder

Default name: `.dvs`. Configurable via `--metadata-folder-name` at init time or by editing `dvs.toml`.

The metadata folder mirrors the project's directory structure. Each tracked file has a `.dvs` sidecar file. For example, `data/input.csv` has metadata at `.dvs/data/input.csv.dvs`.

Contents:
- `<path>.dvs` files: JSON sidecar metadata for each tracked file.
- `.cache/dvs.db`: SQLite hash cache (auto-created, gitignored).

## Metadata file format

Each `.dvs` sidecar is a JSON file:

```json
{
  "hashes": {
    "blake3": "<64-char hex string>"
  },
  "size": 12345,
  "created_by": "username",
  "add_time": "2026-02-27T09:55:51.980811622Z",
  "compression": "zstd",
  "message": "optional user-provided message"
}
```

| Field | Description |
|-------|-------------|
| `hashes.blake3` | Always present. 64-character hex blake3 hash. |
| `size` | File size in bytes. |
| `created_by` | System username at time of `add`. Falls back to `"unknown"`. |
| `add_time` | ISO 8601 timestamp. |
| `compression` | `"zstd"` or `"none"`. Records how this file is stored in the backend. |
| `message` | Omitted from JSON when not provided. |

Two metadata files are considered equal if their `hashes` and `size` match. Other fields are informational.

## Storage layout

Content-addressable: files with identical content share the same storage blob. Files are stored using their blake3 hash split into a 2-character directory prefix and the remaining characters as the filename.

Example: hash `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` is stored at `<storage>/af/1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.

### Compression

Files in storage are compressed with zstd by default. The compression method is recorded per-file in the metadata, so changing the project's compression setting does not affect retrieval of previously added files — `get` reads compression from each file's metadata.

### Atomicity

Writes to storage are atomic: data is written to a temporary file (`.tmp` extension) in the same directory, then renamed into place. Rename is atomic on POSIX within the same filesystem.

### Permissions

Stored files are set read-only after writing. When a group is configured, both the storage directory and stored files are `chown`-ed to that group.

### Storage directory

The storage directory should be dedicated to one project. Sharing storage across projects complicates state management.

The storage directory is not meant to be explored or manipulated directly. Blobs have no file extensions.

## Audit log

Append-only file at `<storage>/audit.log.jsonl`. Written automatically on every successful `add`. Each line is a JSON object:

```json
{
  "operation_id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": 1709035200,
  "user": "alice",
  "file": {
    "path": "data/input.csv",
    "hashes": { "blake3": "af1349..." }
  },
  "action": "add"
}
```

| Field | Description |
|-------|-------------|
| `operation_id` | UUID grouping all files from one `add` invocation. |
| `timestamp` | Unix seconds. |
| `user` | System username. |
| `file.path` | Path relative to project root. |
| `file.hashes` | Hash object (currently only `blake3`). |
| `action` | Currently only `add`. |

Protected by an in-process mutex. No cross-process locking. Audit log failure does not block file operations.

## Hash cache

SQLite database at `{metadata_folder}/.cache/dvs.db`. Caches file hashes keyed by `mtime` and `size` to avoid re-hashing unchanged files.

- Populated by `add` and `get` after successful operations.
- Used by `status` to skip re-hashing.
- Cache directory is automatically added to `.gitignore`.
- If the cache cannot be opened (e.g., corruption), DVS deletes it and retries once. If that also fails, operations proceed without caching.

## Parallelism

`add`, `get`, and `status` run file operations in parallel using rayon.

| Variable | Effect |
|----------|--------|
| `DVS_NUM_THREADS` | Controls thread count. Capped at 16, clamped to number of files. |

## Hashing

blake3 is the only hash algorithm. The `Hashes` struct has an optional `md5` field but it is not currently populated.

## Gitignore

After a successful `add`, each added file is appended to a `.gitignore` in the file's parent directory using the format `/<filename>`, unless already present. If the repository has no `.git` folder, gitignore updates are skipped. Failure to update `.gitignore` is a warning, not an error.
