# TODO

## Plans

### Completed Plans

- [x] **Plan 003: Repo Backend + Fallback Workspace** - Unified repository/workspace interface that prefers Git-backed projects but cleanly falls back to DVS-only workspaces. Implemented `RepoBackend` trait, `GitBackend`, `DvsBackend`, and `Backend` enum with auto-detection.

- [x] **Plan 022: clap CLI Integration** - Built a clap-based CLI (`dvs-cli`) that handles filesystem navigation concerns while delegating all DVS behavior to dvs-core. Includes `init`, `add`, `get`, `status`, and `fs` subcommands.

- [x] **Plan 024: Git-Friendly Remote Data Layout (HTTP-first)** - Manifest-based remote data tracking with push/pull/materialize operations. Implemented `Oid` type with algorithm prefix, `Manifest` and `ManifestEntry` types, `ObjectStore` trait with `LocalStore` and `HttpStore`, `.dvs/` layout helpers, and push/pull/materialize operations.

### Pending Plans

- [ ] **Plan 023: Git Backend (libgit2 default + CLI fallback)** - Git abstraction that defaults to libgit2-backed implementation (`git2` crate) and falls back to Git CLI for edge cases. Covers repo discovery, HEAD info, status, config reading, and lightweight tag creation.

- [ ] **Plan 025: Hashing + Fingerprinting for DVS** - Define hashing and fingerprinting strategy for blobs, tables/dataframes, and trees. Includes algorithm selection (XXH3 vs BLAKE3 vs SHA-256), chunking strategy, Merkle/DAG, and table canonicalization.

- [ ] **Plan 029: fs-err Inclusion** - Adopt the `fs-err` crate for richer filesystem error context in dvs-core (and optionally dvs-cli).

### Future Plans (Not Yet Written)

- [ ] **Plan 026: Wire CLI to new operations** - Connect `dvs push`, `dvs pull`, `dvs materialize` CLI commands to the dvs-core operations.

- [ ] **Plan 027: Server HTTP CAS endpoints** - Implement the HTTP CAS server endpoints (HEAD/GET/PUT for objects) in dvs-server to support remote storage.

- [ ] **Plan 028: R Package Bindings** - Wire dvsR package to dvs-core operations (init, add, get, status, push, pull, materialize).

---

## Implementation Tasks

Note: The current direction uses `.dvs/` + `dvs.lock` for the HTTP-first workflow (see `plans/024-git-friendly-remote-data.md`).

### dvs-cli (Wire new operations)

- [ ] `dvs push [--remote URL]` subcommand - calls `dvs_core::push()`
- [ ] `dvs pull [--remote URL]` subcommand - calls `dvs_core::pull()`
- [ ] `dvs materialize [files...]` subcommand - calls `dvs_core::materialize()`
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

- [x] Unit tests for dvs-core types (102 tests passing)
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
