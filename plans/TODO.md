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

- [x] **Plan 002: Vendor Crates Inclusion** - Vendored miniextendr and all transitive dependencies for CRAN/offline builds. Crates are packaged in `dvsR/inst/vendor.tar.xz` and extracted during configure. dvsR remains isolated from root workspace.

- [x] **Plans 004-021: Crate Inclusion** - All transitive dependencies (miniextendr-api, miniextendr-macros, miniextendr-lint, proc-macro2, quote, syn, unicode-ident, version_check, ahash, cfg-if, getrandom, libc, once_cell, r-efi, wasip2, wit-bindgen, zerocopy, zerocopy-derive) are included in the vendor tarball and used for R package builds.

### In Progress

- [x] **Plan 039: Cross-Interface Consequence Tests** - Shared conformance
  harness to verify CLI/R interfaces produce the same effects.
  - [x] Created `dvs-testkit` crate with `TestRepo`, `WorkspaceSnapshot`
  - [x] Implemented `CoreRunner` baseline
  - [x] Created standard scenarios (init/add/get/status)
  - [x] Implemented `CliRunner` (feature-gated with `cli-runner`)
  - [x] Wired conformance tests into CI (via `--all-features`)
  - [x] Implemented `RRunner` for dvsR (feature: `r-runner`)
    - Spawns Rscript subprocess to execute dvsR functions
    - Captures JSON results and compares workspace snapshots
    - Enable with `DVS_TEST_R=1` environment variable
    - Tests: `test_r_runner_init`, `test_r_runner_add`, `test_conformance_core_vs_r`

### Pending Plans (Documented)

- [ ] **Plan 001: Decoupling Architecture** - `plans/001-decoupling-architecture.md`
- [ ] **Plan 030: Temporal SCD Snapshots for Tabular Data** - `plans/030-temporal-scd-snapshots.md`
- [ ] **Plan 031: Slice Timestamp (As-Of) Views** - `plans/031-slice-ts-asof.md`
- [ ] **Plan 032: Temporal Interlace for Aligned Joins** - `plans/032-temporal-interlace.md`
- [ ] **Plan 033: Audit Logs + Missing Range Detection** - `plans/033-audit-logs-missing-ranges.md`
- [ ] **Plan 034: Concurrency Locks for Dataset Updates** - `plans/034-concurrency-locks.md`
- [ ] **Plan 035: Feature Store + Derived Dataset Framework** - `plans/035-feature-store-derived.md`
- [ ] **Plan 036: Remote Snapshot Sources (HTTP/GitHub)** - `plans/036-remote-snapshot-sources.md`
- [x] **Plan 040: Proc-macro Usage Audit** - Audited proc-macro usage across all crates. serde_derive used in dvs-core/cli/testkit, clap derive in dvs-cli, miniextendr-macros in dvsR. Updated plan with current findings after daemon/server removal.
- [x] **Plan 049: CLI Output Formats (Phase 1)** - Added table output format
  - Added `--format table` option via `tabled` crate
  - Implemented for `dvs status` and `dvs log` commands
  - Pretty ASCII tables with rounded style
  - Phase 2 (optional formats: CSV, YAML, NDJSON, Markdown) deferred

### Future Plans (Not Yet Written)

- [x] **Plan 028: R Package Bindings** - Wire dvsR package to dvs-core operations via miniextendr.
  - [x] Wire `dvs_init()`, `dvs_add()`, `dvs_get()`, `dvs_status()` to dvs-core
  - [x] Wire `dvs_push()`, `dvs_pull()`, `dvs_materialize()` to dvs-core
  - [x] Wire `dvs_log()`, `dvs_rollback()` to dvs-core
  - [x] R-friendly error handling (convert `DvsError` to R errors with `error_type()`)
  - [x] R-friendly return types (data.frames for status, lists for results)
  - Unblocks: Plan 039 `RRunner` implementation

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
- [x] Progress bars for large file operations - Added via `indicatif` crate to add, get, status, push, pull, materialize commands

### dvsR (R Package)

- [x] Wire `dvs_init()` to `dvs_core::init()`
- [x] Wire `dvs_add()` to `dvs_core::add()`
- [x] Wire `dvs_get()` to `dvs_core::get()`
- [x] Wire `dvs_status()` to `dvs_core::status()`
- [x] Wire `dvs_push()` to `dvs_core::push()`
- [x] Wire `dvs_pull()` to `dvs_core::pull()`
- [x] Wire `dvs_materialize()` to `dvs_core::materialize()`
- [x] Wire `dvs_log()` to `dvs_core::log()`
- [x] Wire `dvs_rollback()` to `dvs_core::rollback()`
- [x] R-friendly error handling (convert DvsError to R errors)
- [x] R-friendly return types (data.frames for status via JSON)

---

## Configuration Options

### dvs.yaml / dvs.toml (repository config)

- [x] `storage_dir` - Path to content-addressable storage
- [x] `permissions` - File permissions (octal, e.g., 0640)
- [x] `group` - Unix group for files
- [x] `hash_algo` - Default hash algorithm (blake3, sha256, xxh3)
- [x] `metadata_format` - Metadata file format (json, toml)
- [x] `generated_by` - Version tracking (auto-populated by dvs init)

### .dvs/config.toml (local config)

- [x] `base_url` - Default remote URL for push/pull
- [x] `auth.token` - Bearer token for authentication
- [x] `cache.max_size` - Maximum cache size

Implemented in `dvs-core/src/types/local_config.rs` with `LocalConfig`, `AuthConfig`, and `CacheConfig` structs.

**Wiring:** `push` and `pull` operations now check LocalConfig for `base_url` as a fallback (priority: explicit `--remote` > LocalConfig > manifest).

---

## Testing

- [x] Unit tests for dvs-core types (180 tests passing)
- [x] Unit tests for dvs-core helpers
- [x] Unit tests for dvs-core operations
- [x] Unit tests for dvs-testkit (36 tests: TestRepo, WorkspaceSnapshot, CoreRunner, CliRunner, integration)
- [x] Integration tests with temp directories (14 tests in dvs-testkit/src/integration.rs)
- [x] Integration tests with real git repos (TestRepo uses git2::Repository::init)

### Test workflow note

When running tests with `--all-features`, build the CLI first to ensure feature consistency:

```bash
cargo build -p dvs-cli --all-features && cargo test --all-features
```

This ensures the CLI binary is built with the same features (yaml-config) as the testkit.

- [x] dvsR testthat tests - Added 7 testthat tests covering dvs_init, dvs_add, dvs_get, dvs_status, dvs_log, and message recording. All 27 assertions pass.

---

## Known Issues / Technical Debt

Issues identified during code review (see `reviews/` directory for details).

### High Priority

- [x] **Storage layout mismatch** - ~~External storage uses `{prefix}/{suffix}` but merge/server use `{algo}/{prefix}/{suffix}`. Breaks interop across modules.~~ Fixed: Updated `storage_path_for_hash()` to use `{algo}/{prefix}/{suffix}` layout consistently across all modules (add, get, status, merge, server).

- [x] **Manifest not wired to add** - ~~`dvs add` doesn't update `dvs.lock`, so push/pull/materialize require manual manifest management.~~ Fixed: `dvs add` now creates/updates `dvs.lock` with ManifestEntry for each successfully tracked file (Outcome::Copied or Outcome::Present).

- [x] **CLI file-specific remote ops path resolution** - ~~File arguments to push/pull/materialize resolve to absolute paths, but manifest lookup expects repo-relative paths.~~ Fixed: `push_files`, `pull_files`, and `materialize_files` now convert absolute paths to repo-relative using `pathdiff::diff_paths()` before manifest lookup.

- [x] **Rollback doesn't preserve metadata format** - ~~Rollback always writes JSON (`.dvs`), potentially overwriting TOML metadata.~~ Fixed: Added `format` field to `MetadataEntry` (with serde default for backward compatibility), updated workspace state capture to detect and record metadata format, and updated rollback to use `save_with_format()` and clean up alternate-format files. (`dvs-core/src/types/reflog.rs`, `dvs-core/src/ops/add.rs`, `dvs-core/src/ops/rollback.rs`)

### Medium Priority

- [x] **`dvs add --metadata-format` is no-op** - ~~Flag is validated but never applied.~~ Fixed: Added `add_with_format()` function to dvs-core that accepts optional `MetadataFormat` override. CLI now parses `--metadata-format` and passes it through to override config's default. (`dvs-core/src/ops/add.rs`, `dvs-cli/src/commands/add.rs`)

- [x] **`dvs config set` lacks validation** - ~~`storage_dir` and `group` values aren't validated.~~ Fixed: Added `validate_storage_dir()` that checks path is not empty, not a file if exists, and warns if it doesn't exist yet. Added `validate_group()` that validates group name format and uses `getent` on Unix to verify group exists. (`dvs-cli/src/commands/config.rs`)

- [x] **No CLI for `.dvs/config.toml`** - ~~Users must hand-edit local config for `base_url`/`auth_token`.~~ Fixed: Added `dvs local-config` subcommand with `show`, `get <key>`, `set <key> <value>`, and `unset <key>` actions. Supports `base_url` and `auth_token` keys. Auth token value is not echoed for security. (`dvs-cli/src/commands/local_config.rs`)

- [x] **Rollback `materialize=true` not implemented** - ~~Only restores metadata, never materializes data files.~~ Fixed: When `materialize=true`, rollback now builds OID from metadata (`hash_algo` + `checksum()`), checks if object is cached, and copies from cache to working directory. (`dvs-core/src/ops/rollback.rs`)

### Low Priority

- [x] **HttpStore relies on curl subprocess** - ~~No timeouts or rich error reporting.~~ Fixed: Added 5min operation timeout and 30sec connection timeout to curl commands. (`dvs-core/src/helpers/store.rs:149-157`)

- [x] **Backend normalize doesn't canonicalize** - ~~Only joins with `current_dir`, may mis-handle symlinks/`..`.~~ Fixed: Now uses `fs::canonicalize()` for existing paths and `path_absolutize` for non-existing paths. (`dvs-core/src/helpers/backend.rs:162-185`)

### Test Gaps

- [x] Add tests for non-default hash algorithms (sha256/xxh3) across add/get/status - Added 14 tests covering add/get/status operations with SHA-256 and XXH3 algorithms. Tests are feature-gated with `#[cfg(feature = "sha256")]` and `#[cfg(feature = "xxh3")]` and run with `--features all-hashes`. (`dvs-core/src/ops/add.rs`, `dvs-core/src/ops/get.rs`, `dvs-core/src/ops/status.rs`)
- [x] Add tests for `.dvs.toml` interoperability in get/status/rollback/merge - Added 6 tests: `test_get_with_toml_metadata`, `test_get_toml_metadata_already_present`, `test_status_current_with_toml_metadata`, `test_status_unsynced_with_toml_metadata`, `test_find_tracked_files_with_toml`, `test_rollback_preserves_toml_format`. (`dvs-core/src/ops/get.rs`, `dvs-core/src/ops/status.rs`, `dvs-core/src/ops/rollback.rs`)
- [x] Add tests for manifest-based flows (push/pull/materialize) - Added 4 materialize tests: `test_materialize_from_cache`, `test_materialize_already_up_to_date`, `test_materialize_missing_cache`, `test_materialize_multiple_files`. (`dvs-core/src/ops/materialize.rs`)

---

## Future Features

- [ ] Remote storage backends (S3, GCS, Azure)
- [ ] Compression options (zstd, lz4)
- [ ] Chunking for large files (CDC)
- [ ] Merkle tree for partial sync
- [ ] Git hooks integration (post-checkout, pre-push)
- [ ] Garbage collection for orphaned objects

---

## Remaining Tasks from Code Review (sonnet_reviews/)

See `sonnet_reviews/appendix/findings-by-priority.md` for full details.

### Deferred

- [ ] **P2-7: Init can't bootstrap non-git dir** - Needs design decision on whether to support non-git workspaces

### Pending (Low Priority)

- [ ] **P2-4: Backward compatibility policy** - CLAUDE.md states "no backward compat" but may want formal policy
- [ ] **P2-5: Documentation gaps** - Missing architecture docs, developer onboarding guide
- [x] **P3-1: Batch operations** - Added `--batch` flag to add, get, status, push, pull, materialize commands. Reads paths from stdin (one per line, # comments, empty lines ignored).
- [x] **P3-2: Progress indicators** - Added `indicatif` progress bars to add, get, status, push, pull, materialize commands. Hidden in JSON/quiet mode.
- [ ] **P3-4: Incremental hash verification** - Deferred: Would require storing mtime in metadata and design decisions about fast-path vs full verification
- [ ] **P3-5: Compression support** - Deferred: Feature work requiring new dependencies (zstd/lz4)
- [x] **P3-6: Backend normalize not true canonicalization** - Fixed with `fs::canonicalize()` + `path_absolutize`
- [x] **P3-7: R JSON interface (large integers)** - File sizes > 2^53 (~9 PB) lose precision as f64. Accepted limitation: R has no native 64-bit integers; workaround would require `bit64` package

### Archived (Removed from Project)

- **dvs-daemon** - File watcher daemon removed (was stub only)
- **dvs-server** - HTTP CAS server removed (moved to `archived-crates.zip`)

---

## Misc

- [x] **Optional dvs-core dependencies (Plan 041)**: Making dependencies optional so sibling crates can opt-in.
  - [x] `git2` - via `git2-backend` feature (default on), CLI fallback always available
  - [x] `memmap2` - via `mmap` feature (default on), streaming fallback always available
  - [x] `walkdir` - via `walkdir` feature (default on), recursive fs::read_dir fallback
  - [x] `serde_yaml` - via `yaml-config` feature (default on), JSON fallback uses `dvs.json`
  - [x] `toml` - via `toml-config` feature, uses `dvs.toml` when enabled (without yaml-config)
- [x] The r-package should be named `dvs`, it is just that the _directory_
  is named `dvsR` for convenience.
- [x] add a `install-rpkg` recipe that installs `dvsR`/`{dvs}`, to also mimic
  `just install-cli` as well.
