# `dvs` core crate internals

The `dvs` core crate is the workspace member at `/dvs/`. It is the foundation under both `dvs-cli` and `dvs-rpkg`. It is pure Rust with no FFI.

## 1. High-level mental model

DVS (Data Version Control System) versions large or sensitive files alongside a git repository without committing the file contents to git. The canonical case is binary data such as model weights, datasets, or experiment outputs: files that are too large or too sensitive for git history but still need to be pinned to a specific commit.

The storage model is content-addressed by blake3. On `add`, dvs computes the blake3 hash of the file, writes a (typically zstd-compressed) blob into a backend storage directory under a two-level path `<hash[0:2]>/<hash[2:]>`, and writes a small JSON sidecar (the `.dvs` file) into a parallel metadata tree inside the repo. Git tracks the `.dvs` sidecars and not the blobs, and the original files are auto-gitignored on add. On `get`, dvs reads the `.dvs` sidecar, locates the blob by hash in the backend, and decompresses it back to the original path. Blobs are immutable: they are read-only on disk, never overwritten, and deduplicated by hash across files.

## 2. Module map

All paths below are relative to `/dvs/src/`.

`lib.rs` is the crate entrypoint and public re-export surface. It also hosts the `testutil` module (lines 29-70) used by other modules' unit tests.

`config.rs` defines `Config` (the `dvs.toml` shape), the `Compression` enum with `compress` and `decompress` methods that drive the actual `zstd` I/O, and `Backend` (a serializable enum wrapping `LocalBackend`). `Config::find` walks up from a directory until it locates `dvs.toml`.

`init.rs` implements `init(root_dir, config)`. It writes `dvs.toml`, creates the metadata folder, calls `backend.init()`, and logs an `Init` audit entry. On backend failure it performs best-effort rollback. It does not touch `.gitignore`.

`paths.rs` is the single source of truth for path arithmetic. It defines `DvsPaths` (cwd plus repo root plus metadata folder name), `find_repo_root`, the `AddPathStatus` and `GetPathStatus` enums, and the `CONFIG_FILE_NAME` and `DEFAULT_FOLDER_NAME` constants. `validate_for_add` and `validate_for_get` use `canonicalize` to ensure paths are inside the project. Both accept cwd-relative and repo-root-relative input (fixed in commit `443abcb`).

`backends/mod.rs` declares the `Backend` trait (lines 10-49): `is_initialized`, `init`, `store`, `retrieve`, `exists`, `remove`, `log_audit`, and `read_audit_file`. It is a pure interface.

`backends/local.rs` is `LocalBackend`, the only implementation today. `hash_to_path` (lines 186-193) splits the blake3 hex string at byte 2. `store` writes to a `.tmp` file first and then atomically renames; blobs are made read-only (`0o0440` shared, or `0o0400` private, on Unix). Audit log appends are guarded by a process-scoped mutex (`AUDIT_LOG_LOCK`, line 20) with no cross-process serialization.

`hashes.rs` defines `Hashes { blake3: String, md5: Option<String> }` and the `HashAlg` enum. `compute_from_path` streams the file in 64 KiB chunks (line 29). MD5 is opt-in via the `extra: &[HashAlg]` slice, but no current call site requests it.

`files/metadata.rs` defines `FileMetadata`, which is the on-disk `.dvs` sidecar. Its `save` method (lines 72-191) is the transactional core of `add` and is covered in section 4.

`files/add.rs` defines `add_files`, the public batch add entrypoint. It is rayon-parallel and uses a single `operation_id` per batch.

`files/get.rs` defines `get_files`, which is symmetric to `add`. It includes a post-retrieve integrity re-hash at lines 78-82.

`files/status.rs` defines `get_status`. It walks the metadata folder and computes per-file status in parallel. `StatusFilter` handles user-supplied path scoping.

`files/types.rs` is pure data: `Outcome { Copied, Present }` and `Status { Untracked, Current, Absent, Unsynced }`.

`audit.rs` defines `AuditEntry` and `Action { Add { file, compression }, Init { settings, project_path } }`. `parse_audit_log` reads JSONL with optional path filtering. Note that `Init` entries are excluded whenever a path filter is active (see `audit.rs:81-93`).

`cache.rs` defines `HashCache`, a SQLite-backed stat-keyed hash cache keyed on `path`, `mtime_ns`, and `size` (with values `blake3` and `md5`). It uses WAL mode. Cache errors are fully non-fatal: `try_open_cache` returns `None` on any error.

`gitignore.rs` defines `add_to_gitignore`. It groups paths by parent directory and writes `/filename` entries into the per-directory `.gitignore` co-located with each added file. The function is idempotent and is a no-op if `.git` is absent.

`globbing.rs` defines `resolve_paths_for_add` and `resolve_paths_for_get`. It uses `globset` with `literal_separator(true)`, so `*.csv` does not cross directory boundaries. Users must write `**/*.csv` for recursion.

`progress.rs` defines `ProgressReader<R>` (a `Read` wrapper that calls a callback with byte counts), `FileProgress` (two boxed callbacks, `on_bytes` and `on_done`), and the `OnFileStart` type alias.

`utils.rs` contains `format_size` and `parse_size`, the global atomic thread override (`set_num_threads` and `get_num_threads`), and `get_threadpool`. The thread-count priority is: a manual `set_num_threads()` call wins; if that is unset, the `DVS_NUM_THREADS` environment variable applies; otherwise the count is `available_cpus * 4` capped at 16.

## 3. Public API surface

From `lib.rs:14-24`:

```rust
pub use backends::Backend;
pub use config::Compression;
pub use files::add::{AddDetail, AddResult, add_files};
pub use files::get::{GetDetail, GetResult, get_files};
pub use files::metadata::FileMetadata;
pub use files::status::{FileStatus, StatusDetail, StatusFilter, get_status};
pub use files::types::{Outcome, Status};
pub use hashes::{HashAlg, Hashes};
pub use paths::{AddPathStatus, DvsPaths, find_repo_root};
pub use progress::FileProgress;
pub use utils::{format_size, set_num_threads};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

Notably absent from the re-export wall are `init::init`, `config::Config`, `globbing::*`, and `DvsPaths::from_cwd`. Callers import those directly by path.

The entry points actually invoked by both `dvs-cli` and `dvs-rpkg` are `init`, `add_files`, `get_files`, and `get_status`.

## 4. Core flows

### `dvs init`

The CLI side at `dvs-cli/src/main.rs:193-231` parses `--path` (storage), and optionally `--root-dir`, `--group`, `--metadata-folder-name`, and `--no-compression`. It builds a `Config::new_local(storage_path, group)`. It guards that the storage path is not inside the repo root (via the `abs_storage.starts_with(&abs_root)` check) and then calls `init(&root, config)`.

In `init::init` (`init.rs:13-50`), the function fails if `dvs.toml` already exists, and it fails if `backend.is_initialized()` returns true. (For `LocalBackend`, that check looks for `audit.log.jsonl` in the storage directory; see section 7.) Otherwise, `config.save(root_dir)` writes `dvs.toml`, `fs::create_dir(&metadata_dir)` creates `.dvs/`, and `config.backend().init()` creates the storage directory with the appropriate group and mode. On any failure, the function removes the metadata directory (if it did not pre-exist) and `dvs.toml`. Finally it logs an `AuditEntry::new_init(...)` via `backend.log_audit`.

The on-disk artifacts created by init are: `<root>/dvs.toml`, `<root>/.dvs/`, `<storage>/`, and `<storage>/audit.log.jsonl` containing the init entry.

The `dvs.toml` shape:

```toml
compression = "zstd"
[backend]
path = "/path/to/storage"
group = "mygroup"
```

### `dvs add <path>`

The CLI side at `dvs-cli/src/main.rs:232-302` runs `Config::find`, then `DvsPaths::from_cwd`, then `resolve_paths_for_add`. The last expands globs and directories into a flat list of repo-root-relative paths, which it passes to `add_files`.

In `add_files` (`files/add.rs:90-217`), the flow is as follows. First, `paths.validate_for_add(&files)` canonicalizes `repo_root.join(path)` for each input and rejects NotFound, IsDirectory, and OutsideProject. Then `get_threadpool(matched_paths.len())` builds a rayon pool, and `try_open_cache(paths)` opens (or creates) the SQLite cache at `.dvs/.cache/dvs.db`. A single `operation_id = Uuid::new_v4()` is shared across the whole batch (line 105). Then `par_iter` runs over the validated paths.

For each path, `cache::hashes_for_file` either returns the cached hash (on an mtime+size match) or does a full re-hash. Then `FileMetadata::from_hashes(hashes, size, compression, message)` builds the metadata, and `metadata.save(operation_id, source, backend, paths, rel_path, on_bytes)` does the work. Inside `save`, the fast path is: if both the `.dvs` file and the storage blob already exist and match, skip. Otherwise, `fs::create_dir_all` makes the `.dvs` parent directory, `backend.store(hash, source, compression, on_bytes)` writes a `.tmp`, compresses, and atomically renames into `<storage>/<h[0:2]>/<h[2:]>`, and `fs::write(dvs_file_path, serde_json::to_string_pretty(&self))` writes the JSON sidecar. On partial failure there is two-phase rollback (covered in section 7). Finally, `backend.log_audit(AuditEntry::new_add(...))` records the operation.

After the parallel phase completes, `add_to_gitignore(repo_root, &successful_paths)` runs.

On-disk artifacts: `<storage>/<h[0:2]>/<h[2:]>` (the compressed, read-only blob); `<root>/.dvs/<relative_path>.dvs` (the JSON sidecar); `<root>/<dir>/.gitignore` (updated per-directory); and `<storage>/audit.log.jsonl` (a line appended).

### `dvs get <path>`

The CLI side at `dvs-cli/src/main.rs:405-463` runs `Config::find`, then `DvsPaths::from_cwd`, then `resolve_paths_for_get`. The last scans the metadata tree for matching `.dvs` files and returns repo-root-relative tracked paths, which it passes to `get_files`.

In `get_files` (`files/get.rs:115-198`), `paths.validate_for_get` checks `metadata_path.is_file()` for each input and classifies it as Tracked, NotTracked, or NotFound. Then `par_iter` runs over the validated paths.

For each path, the function reads the sidecar with `serde_json::from_reader(File::open(dvs_file_path))`. It calls `backend.exists(&metadata.hashes)`, which errors if the blob is missing (there is no fallback). If a local file already exists, `cache::hashes_for_file` is called and compared against the sidecar; on a match the function short-circuits with `Outcome::Present`. Otherwise, `backend.retrieve(hash, target_path, compression, on_bytes)` calls `compression.decompress(blob_path, target_path, on_bytes)`. The post-retrieve integrity check at lines 78-82 re-hashes the restored file and compares the blake3 against the sidecar. On a mismatch, the function removes the restored file and returns an error. Finally, the verified hashes are cached.

### `dvs status`

The CLI side at `dvs-cli/src/main.rs:303-404` builds an optional `StatusFilter` from user paths and calls `get_status`.

In `get_status` (`files/status.rs:131-196`), `WalkDir::new(dvs_directory)` enumerates every `.dvs` file in the metadata folder. Then `par_iter` strips the metadata-folder prefix and the `.dvs` extension to recover the repo-relative path and applies `StatusFilter::matches` if a filter is present.

The per-file classification in `get_file_status` is straightforward. If there is no `.dvs` file, the file is `Untracked` (rare here, since we enumerated from the metadata folder). If the `.dvs` file exists but the local file is absent, the file is `Absent`. Otherwise both exist, in which case `cache::hashes_for_file` runs on the local file and is compared against the sidecar to yield `Current` or `Unsynced`.

## 5. Key data structures

`Config` (`config.rs:144-155`):

```rust
pub struct Config {
    compression: Compression,              // "none" | "zstd"
    metadata_folder_name: Option<String>,  // None resolves to ".dvs"
    backend: Backend,                      // serializable backend enum
    cli: Option<CliConfig>,                // progress_threshold (default 500 MB)
}
```

`FileMetadata` (`files/metadata.rs:13-21`), which is the on-disk `.dvs` shape:

```rust
pub struct FileMetadata {
    pub hashes: Hashes,            // blake3 always; md5 optional
    pub size: u64,                 // original file size in bytes
    pub created_by: String,        // username at add time
    pub add_time: jiff::Timestamp, // RFC 3339 in JSON
    pub compression: Compression,  // stored compression used for retrieval
    pub message: Option<String>,
}
```

The `PartialEq` impl for `FileMetadata` (`files/metadata.rs:23-27`) compares only `hashes` and `size`. The `created_by`, `add_time`, and `message` fields are ignored. That equality is what makes the "already present" fast-path in `save` work. It also means re-adding a file with only a new message is silently a no-op.

`AuditEntry` (`audit.rs:32-37`):

```rust
pub struct AuditEntry {
    pub operation_id: String,  // UUID v4, shared across one batch
    pub timestamp: i64,        // unix seconds
    pub user: String,
    pub action: Action,        // Add { file, compression } | Init { settings, project_path }
}
```

`Hashes` (`hashes.rs:16-20`):

```rust
pub struct Hashes {
    pub blake3: String,         // 64-char hex
    pub md5: Option<String>,    // only present if explicitly computed
}
```

`LocalBackend` (`backends/local.rs:79-84`):

```rust
pub struct LocalBackend {
    pub path: PathBuf,        // root of the storage directory
    group: Option<String>,    // Unix group; auto-detected from egid if None
    open: bool,               // if true: 0o2777/0o0666 instead of 0o2770/0o0660
}
```

The error model is `anyhow` everywhere; no typed error enum is exposed. Batch entry points return `anyhow::Result<Vec<...>>`, where individual file failures land in `AddDetail::Error`, `GetDetail::Error`, or `StatusDetail::Error`. Only infrastructure failures (a cache that cannot be opened, a thread pool that cannot be built, or a metadata folder that cannot be read) cause an early return at the batch level.

The `.dvs` sidecar on disk (example, JSON pretty-printed):

```json
{
  "hashes": { "blake3": "af1349b9f5f9a1a6..." },
  "size": 1048576,
  "created_by": "alice",
  "add_time": "2025-05-14T12:00:00Z",
  "compression": "zstd",
  "message": "initial upload"
}
```

## 6. Subsystems

### Hashing

Blake3 is always computed (`hashes.rs:31-32`). MD5 is opt-in via the `extra: &[HashAlg]` slice, but no current caller requests it. The `md5` field and `HashAlg::Md5` exist for legacy and interop use. The hash covers raw file bytes only; no metadata enters the hash input.

### Storage backends

The `Backend` trait (`backends/mod.rs:10-49`) has 8 methods. `LocalBackend` is the only implementation today. The on-disk layout works as follows. The blake3 hex string is split at byte 2, giving 256 top-level buckets. A blob is stored at `<storage>/<h[0:2]>/<h[2:]>`. Blobs are immutable: they are written read-only, never overwritten (store is skipped when `backend.exists()` returns true), and they are only ever removed via the rollback path in `metadata.save`.

### Compression

There are two modes: `Compression::Zstd` (default) and `Compression::None`. The `compress` and `decompress` methods on the enum (`config.rs:46-124`) use `zstd::stream::read::Encoder` and `Decoder` at compression level 0, which is zstd's own default (around level 3). The `ProgressReader` wrapper sits before the encoder, so byte counts reflect pre-compression bytes. That is the size users see in progress bars and the value recorded as `size` in the sidecar. Each sidecar records its compression mode, so `get` decompresses correctly even if the repo's overall config has since changed.

### Audit log

The audit log is a JSONL file at `<storage>/audit.log.jsonl`. Each line is a serialized `AuditEntry`, and the file is append-only. Within one process, writes are serialized by `AUDIT_LOG_LOCK` (`backends/local.rs:20`). Concurrent dvs processes are not serialized, and the code comment is explicit about this. There is no SQLite or structured query layer: `read_audit_file` deserializes the entire log and optionally filters by path via a `HashSet`. `parse_audit_log` skips blank lines and propagates parse errors.

### Cache

The cache is a SQLite database at `<metadata_folder>/.cache/dvs.db`, opened in WAL mode. The schema is `hash_cache(path TEXT PRIMARY KEY, mtime_ns INTEGER, size INTEGER, blake3 TEXT, md5 TEXT)`. The cache is a stat-keyed content hash cache: if a file's mtime and size are unchanged, the cached blake3 is returned without re-reading the file. All cache errors are non-fatal. `try_open_cache` returns `None` on any open error, and operations continue without caching. On first creation, `open_cache` auto-adds the cache directory to `.gitignore` (`cache.rs:138-164`).

### Globbing and paths

Path resolution happens in two phases. First, globbing (`globbing.rs`) expands user inputs (files, directories, and patterns) into a flat set of repo-root-relative paths. Second, validation (`paths.rs` and `DvsPaths`) translates and validates those paths against the project root.

The invariant is that everything reaching `add_files` or `get_files` is repo-root-relative, while user-facing input is cwd-relative. Commit `443abcb` established that `validate_for_add` and `validate_for_get` operate on `DvsPaths::file_path(path)`, which joins `repo_root` (not cwd) with the input. As a result, repo-root-relative paths resolve correctly regardless of the user's cwd.

### Concurrency

All three batch operations build a per-call rayon thread pool via `get_threadpool(work_items)` and dispatch with `par_iter`. The thread count is `min(configured_or_default, work_item_count)`. The default is `available_cpus * 4` capped at 16. Shared state across workers consists of a `Mutex<HashCache>` (the lock is held only for individual lookup or insert calls), a `Mutex<()>` on the audit log, and `Send + Sync` boxed callbacks for progress. `FileProgress::on_bytes` is called from worker threads, and `on_done` is called from the same worker that processed the file.

### Error model

`anyhow` is used everywhere. Batch operations accumulate per-file errors into result variants, which makes partial batch success possible. The CLI exits non-zero if any per-file error occurred. The R package converts `anyhow::Error` to an R condition object via miniextendr (see `dvs-rpkg.md`).

The interesting pattern lives in `metadata.save` (`files/metadata.rs:132-191`): the four-arm match on `(storage_res, metadata_res)` implements an explicit two-phase commit and rollback for the (storage blob, sidecar) pair.

## 7. Non-obvious things

`metadata.save` is the transactional core, not `add_files`. The rollback logic is entirely inside `FileMetadata::save` (`files/metadata.rs:132-191`). When storage succeeds but the sidecar write fails, the blob is removed from the backend. When the sidecar write succeeds but storage failed, the old metadata content is restored. There is no WAL and no fsync ordering: the rollback is best-effort.

The `operation_id` UUID is shared across one batch. `add_files` generates one UUID and passes it to every per-file call (`files/add.rs:105`). An `add` of 50 files produces 50 audit entries with the same `operation_id`. This is intentional batch grouping.

`FileMetadata::eq` ignores provenance. The "already present" fast-path treats two sidecars referring to the same content as equal even if their `created_by`, `add_time`, or `message` differ. Re-adding a file with a different message is therefore a silent no-op.

`is_initialized` keys on `audit.log.jsonl`, not the directory. `LocalBackend::is_initialized` (`backends/local.rs:197-199`) returns true only if the audit log file exists inside the storage directory. That is why `init` can still detect a stale backend even after `.dvs/` and `dvs.toml` have been manually removed.

The gitignore strategy is per-directory. `add_to_gitignore` writes `/filename` entries into the `.gitignore` co-located with each added file rather than into the root `.gitignore`. Adding `data/results/big.bin` writes `/big.bin` into `data/results/.gitignore`. As a result, `dvs add` can modify `.gitignore` files spread across the tree.

The "storage outside repo" check is caller-enforced, not crate-enforced. Both `dvs-cli/src/main.rs:221` and `dvs-rpkg/src/rust/lib.rs:185` reject `abs_storage.starts_with(&abs_root)`. The `dvs::init::init` function itself has no such check.

The cache is gitignored on first creation, not at init time. A first `dvs add` on a repo that has no cache yet silently modifies `.dvs/.gitignore` as a side effect of `open_cache`.

The audit log filter excludes `Init` entries when a path filter is active. In `parse_audit_log` (`audit.rs:81-93`), when `only_files` is non-empty, `Action::Init` entries return false from the filter closure. A query for a specific file will therefore never include the init entry.

Blob deduplication is implicit. Two different files with identical content map to the same hash and the same blob path. The second add writes nothing. Both sidecars reference the same blob, and `get` decompresses the same blob to both target paths. There is no reference counting. `remove` deletes the blob immediately, which is only safe because `remove` is called only from the rollback path in `metadata.save`.

Glob non-recursion is the default. `globset` is configured with `literal_separator(true)`. `*.csv` does not cross directory boundaries, so users must write `**/*.csv` for recursion.

On macOS, `/tmp` is a symlink to `/private/tmp`. `canonicalize` resolves the symlink, which can mismatch `starts_with` checks if one side is canonicalized and the other isn't. The test-side issue was fixed in PR #69.
