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

- [x] **Plan 027: Server HTTP CAS Endpoints** - Implemented HTTP CAS (Content-Addressable Storage) server in `dvs-server`. Includes `HEAD/GET/PUT /objects/{algo}/{hash}` endpoints, `LocalStorage` backend with `{root}/{algo}/{prefix}/{suffix}` layout, API key authentication with permissions (Read/Write/Delete/Admin), and `start_server()` for binding and serving.

- [x] **Plan 045: Track DVS Build/Version in Generated Configs** - Added build-time version tracking to DVS. Implemented `build.rs` in dvs-core that sets `DVS_VERSION`, `DVS_COMMIT_SHA`, and `DVS_VERSION_STRING`. Added `GeneratedBy` struct to `Config` for recording which DVS build created a config file. CLI now shows version with commit hash (e.g., `dvs 0.0.0-9000 (abc12345)`).

- [x] **Plan 043: Replace Axum/Tower with tiny_http for dvs-server** - Migrated dvs-server from async axum/tower stack to synchronous tiny_http. Rewrote api.rs and lib.rs to use tiny_http handlers, updated auth.rs with `*_from_header` functions that accept `Option<&str>` instead of `axum::http::HeaderMap`, updated dvs-testkit server-runner to use thread-based server with `ServerHandle::handle_one()`, removed axum/tower/tower-http from workspace dependencies. All 14 dvs-server tests pass, all 18 dvs-testkit tests pass.

- [x] **Plan 044: Dependency Feature Matrix + Lightweight Defaults** - Changed dvs-core default config from YAML to TOML, made `git2-backend` opt-in, removed unused `rayon` dependency, updated dvs-cli to enable `git2-backend` by default, added yaml-config/toml-config feature passthroughs, updated testkit to use `Config::config_filename()`, replaced axum/tower with tiny_http (Plan 043), updated testkit server-runner.

- [x] **Plan 041: TOML Metadata Files** - Added TOML as alternative metadata format for `.dvs` files. Implemented `MetadataFormat` enum (Json, Toml), multi-format load/save methods (`load_for_data_file()`, `save_with_format()`), `.dvs.toml` file extension for TOML metadata, config option `metadata_format` to control default, and CLI `--metadata-format` flag validation. TOML files (`.dvs.toml`) are preferred if both formats exist.

- [x] **Plan 042: Git Subcommand + Shell Completion Install** - Added `dvs install` command for installing git-status-dvs shim and shell completions, and `dvs uninstall` command to remove them. Added `dvs git-status` command that runs `git status` followed by `dvs status`. Install/uninstall commands support `--install-dir`/`--uninstall-dir` for custom location, `--completions-only` to skip shim, and `--shell` to specify shells (bash, zsh, fish, powershell). Uses `clap_complete` for completion generation.

- [x] **Plan 047: dvs config Command** - Added `dvs config` subcommand for viewing and editing DVS configuration. Includes `dvs config show` (display all values), `dvs config get <key>` (get specific value), and `dvs config set <key> <value>` (set value). Supports keys: storage_dir, permissions, group, hash_algo, metadata_format. Validates values (octal permissions, valid hash algorithms, valid metadata formats). Updates generated_by on save.

- [x] **Plan 046: Merge One DVS Repository into Another** - Added `dvs merge-repo <source>` command to import tracked files, metadata, and objects from a source DVS repository into the destination. Supports `--prefix` to place imports under a subdirectory, `--conflict` mode (abort/skip/overwrite), `--verify` for hash verification, and `--dry-run`. Includes 8 unit tests covering all merge scenarios.

### In Progress

- [ ] **Plan 039: Cross-Interface Consequence Tests** - Shared conformance
  harness to verify CLI/R/other interfaces produce the same effects.
  - [x] Created `dvs-testkit` crate with `TestRepo`, `WorkspaceSnapshot`
  - [x] Implemented `CoreRunner` baseline
  - [x] Created standard scenarios (init/add/get/status)
  - [x] Implemented `CliRunner` (feature-gated with `cli-runner`)
  - [x] Wired conformance tests into CI (via `--all-features`)
  - [x] Implemented `ServerRunner` for HTTP endpoints (feature: `server-runner`)
  - [ ] Implement `RRunner` for dvsR (blocked by Plan 028: R Package Bindings)
    - Needs: dvsR package with working Rust bindings via miniextendr
    - Will spawn R subprocess, execute dvsR functions, capture results
    - Feature: `r-runner` with optional deps on subprocess execution
  - [ ] Implement `DaemonRunner` for daemon IPC (blocked by dvs-daemon implementation)
    - Needs: dvs-daemon with IPC interface (Unix socket or named pipe)
    - Will connect to running daemon, send commands, verify responses
    - Feature: `daemon-runner` with optional deps on dvs-daemon

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

- [ ] **Plan 028: R Package Bindings** - Wire dvsR package to dvs-core operations via miniextendr.
  - Wire `dvs_init()`, `dvs_add()`, `dvs_get()`, `dvs_status()` to dvs-core
  - Wire `dvs_push()`, `dvs_pull()`, `dvs_materialize()` to dvs-core
  - R-friendly error handling (convert `DvsError` to R errors with `error_type()`)
  - R-friendly return types (data.frames for status, lists for results)
  - Blocks: Plan 039 `RRunner` implementation

---

## Implementation Tasks

Note: The current direction uses `.dvs/` + `dvs.lock` for the HTTP-first workflow (see `plans/024-git-friendly-remote-data.md`).

### dvs-cli (Wire new operations)

- [x] `dvs push [--remote URL]` subcommand - calls `dvs_core::push()`
- [x] `dvs pull [--remote URL]` subcommand - calls `dvs_core::pull()`
- [x] `dvs materialize [files...]` subcommand - calls `dvs_core::materialize()`
- [x] `dvs log [-n N]` subcommand - view reflog history
- [x] `dvs rollback [--force] [--no-materialize] <target>` subcommand - rollback to previous state
- [x] `dvs config` subcommand - show/edit configuration
- [ ] `dvs daemon` subcommand - start/stop/status daemon
- [ ] Progress bars for large file operations

### dvs-server (HTTP CAS)

- [x] `HEAD /objects/{algo}/{hash}` - check object existence
- [x] `GET /objects/{algo}/{hash}` - download object
- [x] `PUT /objects/{algo}/{hash}` - upload object (requires Write permission)
- [x] `DELETE /objects/{algo}/{hash}` - delete object (requires Delete permission)
- [x] Authentication middleware (API key / Bearer token)
- [x] Storage backend wiring to LocalStorage
- [x] Auth checks wired into PUT/DELETE handlers
- [x] CORS support (preflight, origin validation, configurable origins)
- [x] Request body size limits (max_upload_size config, 413 response)

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

- [x] Unit tests for dvs-core types (159 tests passing)
- [x] Unit tests for dvs-core helpers
- [x] Unit tests for dvs-core operations
- [x] Unit tests for dvs-server (14 tests: storage, auth)
- [x] Unit tests for dvs-testkit (22 tests: TestRepo, WorkspaceSnapshot, CoreRunner, CliRunner, ServerRunner)
- [ ] Integration tests with temp directories

### Test workflow note

When running tests with `--all-features`, build the CLI first to ensure feature consistency:

```bash
cargo build -p dvs-cli --all-features && cargo test --all-features
```

This ensures the CLI binary is built with the same features (yaml-config) as the testkit.

- [ ] Integration tests with real git repos
- [ ] dvs-daemon IPC tests
- [ ] dvs-server HTTP integration tests
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

## Misc

- [x] **Optional dvs-core dependencies (Plan 041)**: Making dependencies optional so sibling crates can opt-in.
  - [x] `git2` - via `git2-backend` feature (default on), CLI fallback always available
  - [x] `memmap2` - via `mmap` feature (default on), streaming fallback always available
  - [x] `walkdir` - via `walkdir` feature (default on), recursive fs::read_dir fallback
  - [x] `serde_yaml` - via `yaml-config` feature (default on), JSON fallback uses `dvs.json`
  - [x] `toml` - via `toml-config` feature, uses `dvs.toml` when enabled (without yaml-config)
- [ ] The r-package should be named `dvs`, it is just that the _directory_
  is named `dvsR` for convenience.
- [x] add a `install-rpkg` recipe that installs `dvsR`/`{dvs}`, to also mimic
  `just install-cli` as well.
