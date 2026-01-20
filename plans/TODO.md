# TODO

## Plans

### Completed Plans

- [x] **Plan 003: Repo Backend + Fallback Workspace** - Unified repository/workspace interface that prefers Git-backed projects but cleanly falls back to DVS-only workspaces. Implemented `RepoBackend` trait, `GitBackend`, `DvsBackend`, and `Backend` enum with auto-detection.

- [x] **Plan 022: clap CLI Integration** - Built a clap-based CLI (`dvs-cli`) that handles filesystem navigation concerns while delegating all DVS behavior to dvs-core. Includes `init`, `add`, `get`, `status`, and `fs` subcommands.

- [x] **Plan 024: Git-Friendly Remote Data Layout (HTTP-first)** - Manifest-based remote data tracking with push/pull/materialize operations. Implemented `Oid` type with algorithm prefix, `Manifest` and `ManifestEntry` types, `ObjectStore` trait with `LocalStore` and `HttpStore`, `.dvs/` layout helpers, and push/pull/materialize operations.

- [x] **Plan 023: Git Backend (libgit2 default + CLI fallback)** - Git abstraction via `GitOps` trait with `Git2Ops` (libgit2) and `GitCliOps` (system git) implementations. Covers repo discovery, HEAD info, status, config reading, remote URLs, and lightweight tags. Environment variable `DVS_GIT_BACKEND=cli` forces CLI backend.

- [x] **Plan 029: fs-err Inclusion** - Replaced `std::fs` with `fs_err` across dvs-core for better filesystem error messages. All key modules (init, add, get, status, config, layout, store, copy, metadata, materialize, manifest) use `fs_err`. Justfile lint (`check-std-fs`) enforces usage, allowing only `std::fs::Permissions` and `std::fs::Metadata` types.

- [x] **Plan 025: Hashing + Fingerprinting for DVS (Phase 1)** - Multi-algorithm hashing support with BLAKE3 (default), XXH3 (fast), and SHA-256 (interop). Implemented `Hasher` trait for streaming hash computation, feature flags for optional algorithms, configurable `hash_algo` in Config/Metadata, and `dvs add` uses config's hash algorithm.

- [x] **Plan 026: Wire CLI to new operations** - Connected `dvs push`, `dvs pull`, `dvs materialize` CLI commands to dvs-core operations. Each command supports both all-objects and file-specific modes, with progress output.

- [x] **Plan 037: Reversible Workspace Reflog** - Local reflog + snapshot store for DVS-tracked state rollback. Implemented `WorkspaceState` and `ReflogEntry` types, `SnapshotStore` and `Reflog` helpers in dvs-core, wired reflog recording into `dvs add`, and added `dvs log` and `dvs rollback` CLI commands.

- [x] **Plan 038: Exn-First Error Handling Migration** - Migrated dvs-core to `exn`-based error handling. Created `ErrorKind` enum with flat variants and `DvsError` wrapper around `exn::Exn<ErrorKind>`. Preserved stable `error_type()` strings for R interop. Removed `thiserror` and `anyhow` from dvs-core dependencies.

### In Progress
- [ ] **Plan 039: Cross-Interface Consequence Tests** - Shared conformance
  harness to verify CLI/R/other interfaces produce the same effects.
  - [x] Created `dvs-testkit` crate with `TestRepo`, `WorkspaceSnapshot`
  - [x] Implemented `CoreRunner` baseline
  - [x] Created standard scenarios (init/add/get/status)
  - [x] Implemented `CliRunner` (feature-gated with `cli-runner`)
  - [x] Wired conformance tests into CI (via `--all-features`)
  - [ ] Implement `RRunner` for dvsR (blocked by Plan 028)
  - [ ] Implement `ServerRunner` for HTTP endpoints (blocked by Plan 027)
  - [ ] Implement `DaemonRunner` for daemon IPC

### Pending Plans (Documented, Status Unknown)

- [ ] **Plan 001: Decoupling Architecture** - `plans/001-decoupling-architecture.md`
- [ ] **Plan 002: Vendor Crates Inclusion** - `plans/002-vendor-crates-inclusion.md`
- [ ] **Plan 004: miniextendr-api Inclusion** - `plans/004-miniextendr-api-inclusion.md`
- [ ] **Plan 005: miniextendr-macros Inclusion** - `plans/005-miniextendr-macros-inclusion.md`
- [ ] **Plan 006: miniextendr-lint Inclusion** - `plans/006-miniextendr-lint-inclusion.md`
- [ ] **Plan 007: ahash Inclusion** - `plans/007-ahash-inclusion.md`
- [ ] **Plan 008: cfg-if Inclusion** - `plans/008-cfg-if-inclusion.md`
- [ ] **Plan 009: getrandom Inclusion** - `plans/009-getrandom-inclusion.md`
- [ ] **Plan 010: libc Inclusion** - `plans/010-libc-inclusion.md`
- [ ] **Plan 011: once_cell Inclusion** - `plans/011-once_cell-inclusion.md`
- [ ] **Plan 012: proc-macro2 Inclusion** - `plans/012-proc-macro2-inclusion.md`
- [ ] **Plan 013: quote Inclusion** - `plans/013-quote-inclusion.md`
- [ ] **Plan 014: syn Inclusion** - `plans/014-syn-inclusion.md`
- [ ] **Plan 015: unicode-ident Inclusion** - `plans/015-unicode-ident-inclusion.md`
- [ ] **Plan 016: version_check Inclusion** - `plans/016-version_check-inclusion.md`
- [ ] **Plan 017: r-efi Inclusion** - `plans/017-r-efi-inclusion.md`
- [ ] **Plan 018: wasip2 Inclusion** - `plans/018-wasip2-inclusion.md`
- [ ] **Plan 019: wit-bindgen Inclusion** - `plans/019-wit-bindgen-inclusion.md`
- [ ] **Plan 020: zerocopy Inclusion** - `plans/020-zerocopy-inclusion.md`
- [ ] **Plan 021: zerocopy-derive Inclusion** - `plans/021-zerocopy-derive-inclusion.md`
- [ ] **Plan 030: Temporal SCD Snapshots for Tabular Data** - `plans/030-temporal-scd-snapshots.md`
- [ ] **Plan 031: Slice Timestamp (As-Of) Views** - `plans/031-slice-ts-asof.md`
- [ ] **Plan 032: Temporal Interlace for Aligned Joins** - `plans/032-temporal-interlace.md`
- [ ] **Plan 033: Audit Logs + Missing Range Detection** - `plans/033-audit-logs-missing-ranges.md`
- [ ] **Plan 034: Concurrency Locks for Dataset Updates** - `plans/034-concurrency-locks.md`
- [ ] **Plan 035: Feature Store + Derived Dataset Framework** - `plans/035-feature-store-derived.md`
- [ ] **Plan 036: Remote Snapshot Sources (HTTP/GitHub)** - `plans/036-remote-snapshot-sources.md`
- [ ] **Plan 040: Proc-macro Usage Audit** - `plans/040-proc-macro-usage-audit.md`

### Future Plans (Not Yet Written)

- [ ] **Plan 027: Server HTTP CAS endpoints** - Implement the HTTP CAS server endpoints (HEAD/GET/PUT for objects) in dvs-server to support remote storage.

- [ ] **Plan 028: R Package Bindings** - Wire dvsR package to dvs-core operations (init, add, get, status, push, pull, materialize).

---

## Implementation Tasks

Note: The current direction uses `.dvs/` + `dvs.lock` for the HTTP-first workflow (see `plans/024-git-friendly-remote-data.md`).

### dvs-cli (Wire new operations)

- [x] `dvs push [--remote URL]` subcommand - calls `dvs_core::push()`
- [x] `dvs pull [--remote URL]` subcommand - calls `dvs_core::pull()`
- [x] `dvs materialize [files...]` subcommand - calls `dvs_core::materialize()`
- [x] `dvs log [-n N]` subcommand - view reflog history
- [x] `dvs rollback [--force] [--no-materialize] <target>` subcommand - rollback to previous state
- [ ] `dvs config` subcommand - show/edit configuration
- [ ] `dvs daemon` subcommand - start/stop/status daemon
- [ ] Progress bars for large file operations

### dvs-server (HTTP CAS)

- [ ] `HEAD /objects/{algo}/{hash}` - check object existence
- [ ] `GET /objects/{algo}/{hash}` - download object
- [ ] `PUT /objects/{algo}/{hash}` - upload object
- [ ] Authentication middleware (API key / Bearer token)
- [ ] Storage backend wiring to LocalStorage

### dvs-daemon

- [ ] `start_daemon()` - Initialize and run event loop
- [ ] `stop_daemon()` - Graceful shutdown
- [ ] File watcher integration with notify crate
- [ ] Event handler for auto-add/auto-sync
- [ ] IPC client/server for daemon control
- [ ] PID file and signal handling

### dvsR (R Package)

- [ ] Wire `dvs_init()` to `dvs_core::init()`
- [ ] Wire `dvs_add()` to `dvs_core::add()`
- [ ] Wire `dvs_get()` to `dvs_core::get()`
- [ ] Wire `dvs_status()` to `dvs_core::status()`
- [ ] Wire `dvs_push()` to `dvs_core::push()`
- [ ] Wire `dvs_pull()` to `dvs_core::pull()`
- [ ] Wire `dvs_materialize()` to `dvs_core::materialize()`
- [ ] R-friendly error handling (convert DvsError to R errors)
- [ ] R-friendly return types (data.frames for status)

---

## Configuration Options

### dvs.yaml (repository config)

- [ ] `storage_dir` - Path to content-addressable storage
- [ ] `permissions` - File permissions (octal, e.g., 0640)
- [ ] `group` - Unix group for files
- [ ] `hash_algorithm` - Default hash algorithm (blake3, sha256, xxh3)

### .dvs/config.toml (local config)

- [ ] `base_url` - Default remote URL for push/pull
- [ ] `auth.token` - Bearer token for authentication
- [ ] `cache.max_size` - Maximum cache size

### Daemon config

- [ ] `watch_paths` - Directories to watch
- [ ] `debounce_ms` - Delay before processing changes
- [ ] `auto_add` - Enable auto-add for new files
- [ ] `auto_sync` - Enable auto-sync for changes

### Server config

- [ ] `host` - Bind address
- [ ] `port` - Listen port
- [ ] `storage_root` - Storage directory
- [ ] `auth.enabled` - Enable authentication
- [ ] `auth.api_keys` - List of API keys

---

## Testing

- [x] Unit tests for dvs-core types (142 tests passing)
- [x] Unit tests for dvs-core helpers
- [x] Unit tests for dvs-core operations
- [ ] Integration tests with temp directories
- [ ] Integration tests with real git repos
- [ ] dvs-daemon IPC tests
- [ ] dvs-server API tests
- [ ] dvsR testthat tests

---

## Future Features

- [ ] Remote storage backends (S3, GCS, Azure)
- [ ] Compression options (zstd, lz4)
- [ ] Chunking for large files (CDC)
- [ ] Merkle tree for partial sync
- [ ] Git hooks integration (post-checkout, pre-push)
- [ ] Garbage collection for orphaned objects
- [ ] Web UI for server

## Misc.

- [ ] **Optional dvs-core dependencies (Plan 041)**: Making dependencies optional so sibling crates can opt-in.
  - [x] `git2` - via `git2-backend` feature (default on), CLI fallback always available
  - [x] `memmap2` - via `mmap` feature (default on), streaming fallback always available
  - [ ] `serde_yaml` - via `yaml-config` feature
  - [ ] `walkdir` - via `walkdir` feature
