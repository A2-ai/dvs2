# `dvs audit`

Append-only log of all `add` operations, stored in the backend.

## Current status

The audit log is an internal mechanism. There is no `dvs audit` CLI command or R function yet. The log is written automatically by `add` and can be read programmatically via the Rust library. A user-facing command is planned but not implemented.

## Behavior

Every successful `add` appends one entry per file to `audit.log.jsonl` in the storage directory. The log is append-only and is never truncated by dvs.

### Entry format

Each line is a JSON object:

```json
{
  "operation_id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": 1709035200,
  "user": "alice",
  "file": {
    "path": "data/input.csv",
    "hashes": {
      "blake3": "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    }
  },
  "action": "add"
}
```

- `operation_id`: UUID grouping all files from one `add` invocation.
- `timestamp`: unix seconds.
- `user`: system username of whoever ran the command.
- `file.path`: path relative to project root.
- `file.hashes`: hash object (currently only `blake3`).
- `action`: currently only `add`.

### Concurrency

The audit log is protected by an in-process mutex. A single `dvs` process will not corrupt the log even under parallel file processing. There is no protection against multiple concurrent `dvs` processes appending to the same log.

### Failure handling

If the audit log write fails, the error is logged but the `add` operation itself is not rolled back. The audit log is informational; its failure does not block file operations.

## Rust library

```rust
pub struct AuditEntry {
    pub operation_id: String,
    pub timestamp: i64,
    pub user: String,
    pub file: AuditFile,
    pub action: Action,
}

pub struct AuditFile {
    pub path: PathBuf,
    pub hashes: Hashes,
}

pub enum Action {
    Add,
}
```

### Writing

`AuditEntry::new_add(operation_id, file)` constructs an entry with the current timestamp and username. Writing is done by the backend's `log_audit` method.

### Reading

```rust
pub fn parse_audit_log(
    reader: impl BufRead,
    only_files: &HashSet<PathBuf>,
) -> Result<Vec<AuditEntry>>
```

Reads a JSONL stream and optionally filters to entries matching the given file paths. An empty `only_files` set returns all entries.

The backend also exposes `read_audit_file(files)` which opens the log file and delegates to `parse_audit_log`.

## Proposed CLI (not implemented)

```
dvs audit [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...              Optional file paths to filter audit entries

Options:
      --json              Output as JSON
  -h, --help              Print help
```

## Proposed R package (not implemented)

```r
dvs_audit(files = character())
```
