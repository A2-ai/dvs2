# DONE

## Project Setup

- [x] Initialize Cargo workspace with resolver = "3"
- [x] Create dvs-core crate with lib structure
- [x] Create dvs-cli crate (binary scaffold)
- [x] Create dvs-daemon crate with async support
- [x] Create dvs-server crate with axum
- [x] Configure workspace.package inheritance
- [x] Configure workspace.dependencies with scoping comments
- [x] Add descriptions to all crate Cargo.toml files
- [x] Setup dvsR package directory structure
- [x] Configure miniextendr for R-Rust bindings
- [x] Fix PACKAGE_NAME case sensitivity in configure.ac
- [x] Create justfile with build recipes

## Architecture & Planning

- [x] Copy design docs from reference dvs repo
- [x] Create plans/001-decoupling-architecture.md
- [x] Define crate responsibilities and boundaries
- [x] Design module structure for each crate

## dvs-core Scaffolding

- [x] Create src/lib.rs with module exports
- [x] Create src/types/mod.rs
- [x] Create src/types/config.rs - Config struct with fields
- [x] Create src/types/metadata.rs - Metadata struct with fields
- [x] Create src/types/file_info.rs - FileInfo struct
- [x] Create src/types/outcome.rs - AddResult, GetResult, StatusResult
- [x] Create src/types/error.rs - DvsError enum with thiserror
- [x] Create src/ops/mod.rs
- [x] Create src/ops/init.rs - init() signature and helpers
- [x] Create src/ops/add.rs - add() signature and helpers
- [x] Create src/ops/get.rs - get() signature and helpers
- [x] Create src/ops/status.rs - status() signature and helpers
- [x] Create src/helpers/mod.rs
- [x] Create src/helpers/hash.rs - blake3 hashing signatures
- [x] Create src/helpers/copy.rs - file copy signatures
- [x] Create src/helpers/file.rs - file metadata signatures
- [x] Create src/helpers/config.rs - config loading signatures
- [x] Create src/helpers/parse.rs - glob/path parsing signatures
- [x] Create src/helpers/repo.rs - git repo utilities signatures
- [x] Create src/helpers/ignore.rs - .dvsignore signatures
- [x] Create src/helpers/cache.rs - hash cache signatures

## dvs-daemon Scaffolding

- [x] Create src/lib.rs with module exports and DaemonError
- [x] Create src/watcher.rs - FileWatcher, FileEvent, WatcherConfig
- [x] Create src/handler.rs - EventHandler with event routing
- [x] Create src/ipc.rs - DaemonClient, DaemonServer, commands
- [x] Create src/config.rs - DaemonConfig struct

## dvs-server Scaffolding

- [x] Create src/lib.rs with module exports and ServerError
- [x] Create src/api.rs - REST endpoints, AppState, response types
- [x] Create src/auth.rs - AuthConfig, ApiKey, Permission, AuthContext
- [x] Create src/storage.rs - StorageBackend trait, LocalStorage
- [x] Create src/config.rs - ServerConfig struct

## Documentation

- [x] Create CLAUDE.md with project context
- [x] Create TODO.md (this file's companion)
- [x] Create DONE.md (this file)

## Plan 003: Repo Backend + Fallback Workspace

Implemented a unified repository/workspace interface that prefers Git-backed projects
but cleanly falls back to DVS-only workspaces when no Git repo exists.

### helpers/ignore.rs - Gitignore-style pattern matching

- [x] `IgnoreSource` enum (GitIgnore, DvsIgnore, Ignore) for tracking pattern origins
- [x] `IgnorePattern` struct with compiled glob matching, negation, and dir-only support
- [x] `IgnorePatterns` collection with `add_from_file()` and `is_ignored()` methods
- [x] Helper functions: `load_gitignore_patterns()`, `load_dvs_ignore_patterns()`
- [x] Helper functions: `add_gitignore_pattern()`, `add_dvsignore_pattern()`, `add_ignore_pattern()`
- [x] `should_ignore()` convenience function for simple pattern checking

### helpers/backend.rs - Repository backend abstraction

- [x] `RepoBackend` trait with methods: `root()`, `normalize()`, `add_ignore()`, `is_ignored()`, `current_branch()`, `backend_type()`
- [x] `GitBackend` implementation:
  - Root detection via `.git` directory
  - Path normalization using pathdiff
  - Gitignore handling via `load_gitignore_patterns()`
  - Branch detection by reading `.git/HEAD`
- [x] `DvsBackend` implementation:
  - Root detection via `dvs.yaml` or `.dvs/` directory
  - Path normalization using pathdiff
  - DVS ignore handling (`.dvsignore` and `.ignore`)
  - Returns `None` for `current_branch()` (no branch concept)
- [x] `Backend` enum for runtime dispatch (Git or Dvs)
- [x] `detect_backend()` - prefers Git, falls back to DVS-only workspace
- [x] `detect_backend_cwd()` - convenience wrapper using current directory

### ops updates - Backend integration

- [x] `init()` now calls `detect_backend_cwd()`, added `init_with_backend()`
- [x] `add()` now calls `detect_backend_cwd()`, added `add_with_backend()`
- [x] `get()` now calls `detect_backend_cwd()`, added `get_with_backend()`
- [x] `status()` now calls `detect_backend_cwd()`, added `status_with_backend()`

### Tests - 22 tests passing

- [x] Pattern parsing tests (simple, negated, directory, path, comments, empty)
- [x] Pattern matching tests (basename, full path, directory-only)
- [x] Collection tests (negation handling, should_ignore helper)
- [x] Backend tests (types, root finding, branch detection, path normalization)
- [x] DVS workspace detection tests (`dvs.yaml` and `.dvs/` directory)

## Plan 022: clap CLI Integration

Built a clap-based CLI that handles filesystem navigation concerns while delegating
all DVS behavior to dvs-core.

### main.rs - CLI entry point with clap parsing

- [x] `Cli` struct with global options (`-C/--cwd`, `--repo`, `--format`, `--quiet`)
- [x] `Command` enum with subcommands: `Init`, `Add`, `Get`, `Status`, `Fs`
- [x] `FsCommand` for navigation helpers: `Pwd`, `Ls`
- [x] `OutputFormat` enum: `Human`, `Json`
- [x] Working directory change via `-C` flag before command dispatch
- [x] Proper exit codes (success/failure)

### commands/ - Command implementations

- [x] `commands/mod.rs` - `CliError` type with thiserror, `Result` alias
- [x] `commands/init.rs` - Parse permissions, resolve paths, call `dvs_core::init()`
- [x] `commands/add.rs` - Resolve file paths, call `dvs_core::add()`, output results by outcome
- [x] `commands/get.rs` - Resolve file paths, call `dvs_core::get()`, output results by outcome
- [x] `commands/status.rs` - Resolve file paths, call `dvs_core::status()`, output by file status

### output.rs - Output formatting

- [x] `Output` struct with format and quiet mode
- [x] Methods: `println()`, `success()`, `info()`, `warn()`, `error()`
- [x] ANSI color codes for human format (green success, yellow warning, red error)
- [x] JSON output format support
- [x] `escape_json()` helper for safe JSON string output

### paths.rs - Path resolution utilities

- [x] `set_cwd()` - Change working directory with validation
- [x] `resolve_path()` - Expand `~` and make paths absolute
- [x] `normalize_path()` - Resolve `.` and `..` components
- [x] `home_dir()` - Cross-platform home directory detection

### CLI Features

- [x] Global `-C/--cwd` flag to change directory before running
- [x] Global `--repo` flag for explicit repository root
- [x] Global `--format` flag (human/json output)
- [x] Global `-q/--quiet` flag to suppress non-error output
- [x] `dvs init <storage_dir> [--permissions] [--group]`
- [x] `dvs add <files...> [-m/--message]`
- [x] `dvs get <files...>`
- [x] `dvs status [files...]`
- [x] `dvs fs pwd` - Print current directory
- [x] `dvs fs ls [path]` - List directory contents

### Tests - 6 tests passing

- [x] Permission parsing tests (valid octal, invalid input)
- [x] JSON escaping test
- [x] Path normalization test
- [x] Absolute path resolution test
- [x] Tilde expansion test

## dvs-core Business Logic Implementation

Full implementation of the core DVS business logic with 66 tests passing.

### helpers/hash.rs - Blake3 hashing utilities

- [x] `MMAP_THRESHOLD` constant (16KB) for choosing hashing strategy
- [x] `get_file_hash()` - Auto-selects mmap vs buffered read based on file size
- [x] `storage_path_for_hash()` - Computes content-addressable storage path (`{prefix}/{suffix}`)
- [x] `hash_mmap()` - Memory-mapped hashing for large files
- [x] `hash_read()` - Buffered read hashing for small files
- [x] `verify_hash()` - Compare file hash against expected value
- [x] 4 unit tests for hashing functionality

### helpers/copy.rs - File copy utilities

- [x] `COPY_BUFFER_SIZE` constant (64KB) for buffered copying
- [x] `copy_to_storage()` - Copy file to storage with optional permissions/group
- [x] `copy_from_storage()` - Copy file from storage to local path
- [x] `copy_file()` - Buffered file copy implementation
- [x] `set_permissions()` - Unix file permissions via libc (with non-Unix stub)
- [x] `set_group()` - Unix group ownership via libc/chown (with non-Unix stub)
- [x] `group_exists()` - Check if group exists on system
- [x] 3 unit tests for copy functionality

### helpers/file.rs - File metadata utilities

- [x] `save_metadata()` - Save metadata to `.dvs` file
- [x] `load_metadata()` - Load metadata from `.dvs` file
- [x] `check_meta_files_exist()` - Batch check for missing metadata
- [x] `get_current_username()` - Cross-platform username detection (env vars + libc)
- [x] `get_file_size()` - Get file size in bytes
- [x] `file_exists()` - Check if file exists
- [x] `metadata_path_for()` - Get metadata path for data file
- [x] `data_path_for()` - Get data path from metadata file
- [x] 5 unit tests for file utilities

### helpers/config.rs - Configuration utilities

- [x] `find_repo_root()` - Search upward for .git, dvs.yaml, or .dvs/
- [x] `find_repo_root_from()` - Same, from a specific start path
- [x] `load_config()` - Load Config from dvs.yaml
- [x] `save_config()` - Save Config to dvs.yaml
- [x] `validate_storage_dir()` - Check directory exists and is writable
- [x] `create_storage_dir()` - Create storage directory (with parents)
- [x] `config_path()` - Get path to dvs.yaml
- [x] `is_initialized()` - Check if DVS is initialized
- [x] `expand_path()` - Expand `~` and relative paths
- [x] 6 unit tests for config utilities

### types/config.rs - Config load/save

- [x] `Config::load()` - Load from YAML file using serde_yaml
- [x] `Config::save()` - Save to YAML file
- [x] 3 unit tests for config serialization

### types/metadata.rs - Metadata load/save

- [x] `Metadata::load()` - Load from JSON file using serde_json
- [x] `Metadata::save()` - Save to JSON file (pretty-printed)
- [x] 5 unit tests for metadata serialization

### ops/init.rs - Initialization operation

- [x] `init()` - Initialize DVS with backend auto-detection
- [x] `init_with_backend()` - Full init implementation:
  - Validate group membership
  - Create/validate storage directory
  - Create Config
  - Check for existing config (ConfigMismatch error)
  - Save dvs.yaml
  - Add `*.dvs` to .gitignore
- [x] `setup_storage_directory()` - Create or validate storage dir
- [x] `validate_group()` - Check group exists
- [x] `add_to_gitignore()` - Add pattern to .gitignore (idempotent)
- [x] 5 unit tests for init functionality

### ops/add.rs - Add files operation

- [x] `add()` - Add files with backend auto-detection
- [x] `add_with_backend()` - Full add implementation:
  - Load config
  - Expand glob patterns
  - Process each file
- [x] `expand_globs()` - Expand glob patterns, filter ignored files
- [x] `add_single_file()` - Add a single file:
  - Compute relative path
  - Check file exists
  - Get file size
  - Compute blake3 hash
  - Check if already present (Outcome::Present)
  - Copy to storage
  - Create and save metadata
- [x] `rollback_add()` - Cleanup on failure
- [x] 5 unit tests for add functionality

### ops/get.rs - Retrieve files operation

- [x] `get()` - Get files with backend auto-detection
- [x] `get_with_backend()` - Full get implementation:
  - Load config
  - Expand glob patterns to tracked files
  - Process each file
- [x] `expand_globs_tracked()` - Expand patterns to files with .dvs metadata
- [x] `get_single_file()` - Get a single file:
  - Compute relative path
  - Load metadata
  - Check if local matches (Outcome::Present)
  - Verify storage file exists
  - Copy from storage
  - Verify hash after copy
- [x] `file_matches_metadata()` - Check if file matches expected hash
- [x] 3 unit tests for get functionality

### ops/status.rs - Status check operation

- [x] `status()` - Check status with backend auto-detection
- [x] `status_with_backend()` - Full status implementation:
  - Load config
  - Find files (all tracked or expand patterns)
  - Process each file
- [x] `find_all_tracked_files()` - Walk repo and find all .dvs files
- [x] `expand_patterns()` - Expand patterns to tracked files
- [x] `status_single_file()` - Check status of single file:
  - Compute relative path
  - Load metadata
  - Determine status (Current/Absent/Unsynced)
  - Verify storage file exists
- [x] `determine_status()` - Compare local file to metadata hash
- [x] 4 unit tests for status functionality

### Summary

- **66 tests passing** (up from 28)
- All dvs-core helpers fully implemented
- All dvs-core operations fully implemented
- Full integration with Backend abstraction
- Cross-platform support (Unix permissions with non-Unix stubs)

## Plan 024: Git-Friendly Remote Data Layout (HTTP-first)

Implemented manifest-based remote data tracking with push/pull/materialize operations
for HTTP content-addressable storage.

### types/oid.rs - Object ID with algorithm prefix

- [x] `HashAlgo` enum (Blake3, Sha256, Xxh3) with prefix strings and hex lengths
- [x] `Oid` struct with `algo` and `hex` fields
- [x] `Oid::new()`, `Oid::blake3()`, `Oid::sha256()`, `Oid::xxh3()` constructors
- [x] `Oid::parse()` - Parse from "algo:hex" string format
- [x] `Oid::storage_subpath()` - Returns "{algo}/{prefix}/{suffix}" for storage
- [x] `Oid::storage_path_components()` - Returns (prefix, suffix) tuple
- [x] Custom serde serialization/deserialization for "algo:hex" format
- [x] `Display` and `FromStr` implementations
- [x] 9 unit tests for OID functionality

### types/manifest.rs - Manifest file (dvs.lock) tracking

- [x] `Compression` enum (None, Zstd, Gzip)
- [x] `ManifestEntry` struct with path, oid, bytes, compression, remote fields
- [x] `Manifest` struct with version, base_url, entries
- [x] `Manifest::new()`, `Manifest::filename()` - Constructor and filename constant
- [x] `Manifest::load()`, `Manifest::save()` - JSON serialization
- [x] `Manifest::add()`, `Manifest::upsert()`, `Manifest::remove()`, `Manifest::get()`
- [x] `Manifest::merge()` - Merge another manifest
- [x] `Manifest::unique_oids()` - Get deduplicated OIDs
- [x] 9 unit tests for manifest functionality

### helpers/store.rs - Object store abstraction

- [x] `ObjectStore` trait with `has()`, `get()`, `put()`, `store_type()` methods
- [x] `LocalStore` implementation:
  - Filesystem-based content-addressable storage
  - `object_path()` - Returns full path for OID
  - Idempotent put (skips if exists)
- [x] `HttpStore` implementation:
  - HTTP CAS client using curl subprocess
  - `object_url()` - Returns "{base}/objects/{algo}/{hex}"
  - HEAD for existence check, GET for download, PUT for upload
- [x] `ChainStore` implementation:
  - Multi-store with fallback
  - `has()` checks all stores in order
  - `get()` fetches from first store that has object
  - `put()` writes to all stores
- [x] 6 unit tests for store functionality

### helpers/layout.rs - Local .dvs/ directory structure

- [x] `DVS_DIR` constant (".dvs")
- [x] `Layout` struct with repo_root path
- [x] Path methods: `dvs_dir()`, `config_path()`, `cache_dir()`, `objects_dir()`
- [x] Path methods: `state_dir()`, `locks_dir()`, `manifest_path()`, `lock_path()`
- [x] `cached_object_path()` - Returns path for cached OID
- [x] `init()` - Create .dvs/ directory structure
- [x] `exists()`, `is_cached()` - Check directory/object existence
- [x] `cached_oids()` - Walk cache and return all stored OIDs
- [x] `MaterializedState` struct with files map and timestamp
- [x] `MaterializedState::load()`, `save()`, `needs_materialize()`, `mark_materialized()`
- [x] 8 unit tests for layout functionality

### ops/push.rs - Upload objects to remote

- [x] `PushResult` struct with oid, uploaded flag, error option
- [x] `PushResult::success()`, `error()`, `is_error()` methods
- [x] `PushSummary` struct with uploaded/present/failed counts and results
- [x] `push()` - Push with backend auto-detection
- [x] `push_with_backend()` - Full push implementation:
  - Load manifest
  - Determine remote URL (arg or manifest base_url)
  - Create local/remote stores
  - Push each unique OID
- [x] `push_single_object()` - Push one object (skip if present on remote)
- [x] `push_files()` - Push specific files by path
- [x] 3 unit tests for push functionality

### ops/pull.rs - Download objects from remote

- [x] `PullResult` struct with oid, downloaded flag, error option
- [x] `PullResult::success()`, `error()`, `is_error()` methods
- [x] `PullSummary` struct with downloaded/cached/failed counts and results
- [x] `pull()` - Pull with backend auto-detection
- [x] `pull_with_backend()` - Full pull implementation:
  - Load manifest
  - Determine remote URL
  - Initialize local cache
  - Pull each unique OID
- [x] `pull_single_object()` - Pull one object (skip if already cached)
- [x] `pull_files()` - Pull specific files by path
- [x] 3 unit tests for pull functionality

### ops/materialize.rs - Copy cached objects to working tree

- [x] `MaterializeResult` struct with path, oid, materialized flag, error option
- [x] `MaterializeResult::success()`, `error()`, `is_error()` methods
- [x] `MaterializeSummary` struct with materialized/up_to_date/failed counts and results
- [x] `materialize()` - Materialize with backend auto-detection
- [x] `materialize_with_backend()` - Full materialize implementation:
  - Load manifest and materialized state
  - Materialize each entry
  - Save updated state
- [x] `materialize_single_file()` - Materialize one file:
  - Check if already materialized with same OID
  - Copy from cache to working tree
  - Update materialized state
- [x] `materialize_files()` - Materialize specific files by path
- [x] 3 unit tests for materialize functionality

### lib.rs exports

- [x] Re-export `Oid`, `HashAlgo`, `Manifest`, `ManifestEntry`, `Compression`
- [x] Re-export `push`, `push_with_backend`, `push_files`, `PushResult`, `PushSummary`
- [x] Re-export `pull`, `pull_with_backend`, `pull_files`, `PullResult`, `PullSummary`
- [x] Re-export `materialize`, `materialize_with_backend`, `materialize_files`, `MaterializeResult`, `MaterializeSummary`

### Summary

- **102 tests passing** (up from 66)
- Manifest-based remote data tracking via dvs.lock
- Content-addressable OID format with algorithm prefix (blake3:, sha256:, xxh3:)
- ObjectStore trait with local, HTTP, and chain store implementations
- Local .dvs/ cache directory structure
- Push/pull/materialize operations for remote data workflow
- MaterializedState tracking for incremental updates

## Plan 023: Git Backend (libgit2 default + CLI fallback)

Implemented a Git abstraction layer that defaults to libgit2 (`git2` crate) and falls back to the system Git CLI for edge cases or when explicitly requested.

### Dependencies

- [x] Added `git2 = "0.20"` to workspace dependencies
- [x] Added `git2.workspace = true` to dvs-core dependencies

### helpers/git_ops.rs - Git operations abstraction

- [x] `HeadInfo` struct with `oid`, `branch`, `is_detached` fields
- [x] `StatusInfo` struct with `is_dirty`, `has_untracked` fields
- [x] `GitOps` trait with methods:
  - `discover_repo_root()` - Find repository root from path
  - `head_info()` - Get HEAD commit OID and branch name
  - `status_info()` - Get dirty/untracked status
  - `config_value()` - Read Git config values
  - `remote_url()` - Get remote URL
  - `create_tag_lightweight()` - Create lightweight tag
  - `backend_name()` - Get backend identifier

### Git2Ops implementation (libgit2)

- [x] `Git2Ops::new()` - Create new git2 backend
- [x] `discover_repo_root()` - Uses `Repository::discover()`
- [x] `head_info()` - Reads HEAD, handles detached state and unborn branches
- [x] `status_info()` - Uses `StatusOptions` with untracked file detection
- [x] `config_value()` - Reads from repository config
- [x] `remote_url()` - Uses `find_remote()` to get URL
- [x] `create_tag_lightweight()` - Creates lightweight tag via `tag_lightweight()`

### GitCliOps implementation (system git)

- [x] `GitCliOps::new()` - Create new CLI backend
- [x] `run_git()` - Execute git command with `-C` flag
- [x] `run_git_optional()` - Execute git command, return None on failure
- [x] `discover_repo_root()` - Uses `git rev-parse --show-toplevel`
- [x] `head_info()` - Uses `git rev-parse HEAD` and `git symbolic-ref --short HEAD`
- [x] `status_info()` - Parses `git status --porcelain` output
- [x] `config_value()` - Uses `git config --get`
- [x] `remote_url()` - Uses `git remote get-url`
- [x] `create_tag_lightweight()` - Uses `git tag`

### Backend selection

- [x] `select_git_backend()` - Returns CLI backend if `DVS_GIT_BACKEND=cli`, otherwise git2
- [x] `default_git_backend()` - Returns `Git2Ops`
- [x] `cli_git_backend()` - Returns `GitCliOps`
- [x] `with_fallback()` - Tries git2 first, falls back to CLI on certain errors
- [x] `should_fallback()` - Checks if error message suggests fallback (unsupported, worktree, submodule, sparse)

### Integration with existing GitBackend

- [x] Updated `GitBackend::find_root()` to use `select_git_backend().discover_repo_root()`
- [x] Added `GitBackend::find_root_simple()` as filesystem-only fallback
- [x] Updated `GitBackend::current_branch()` to use `git_ops.head_info()`

### Tests - 14 new tests

- [x] `test_head_info_default` - HeadInfo default values
- [x] `test_status_info_default` - StatusInfo default values
- [x] `test_git2_backend_name` - Git2Ops backend name
- [x] `test_cli_backend_name` - GitCliOps backend name
- [x] `test_select_default_backend` - Default backend selection
- [x] `test_should_fallback` - Fallback error detection
- [x] `test_git2_discover_repo_root` - Git2 repo discovery
- [x] `test_git2_head_info` - Git2 HEAD info
- [x] `test_git2_status_info` - Git2 status info
- [x] `test_git2_config_value` - Git2 config reading
- [x] `test_git2_remote_url` - Git2 remote URL
- [x] `test_cli_discover_repo_root` - CLI repo discovery
- [x] `test_cli_head_info` - CLI HEAD info
- [x] `test_with_fallback` - Fallback mechanism

### Summary

- **116 tests passing** (up from 102)
- Git operations abstraction via `GitOps` trait
- Default libgit2 backend (`git2` crate) for performance
- CLI fallback for edge cases (worktrees, submodules, sparse checkouts)
- Environment variable `DVS_GIT_BACKEND=cli` to force CLI backend
- Automatic fallback on unsupported repository layouts
- Integrated with existing `GitBackend` for seamless usage

## Plan 029: fs-err Inclusion

Replaced `std::fs` with `fs_err` crate across dvs-core for improved filesystem error messages that include paths.

### Dependencies

- [x] Added `fs-err = "2"` to workspace dependencies

### Module updates

- [x] `helpers/config.rs` - `use fs_err as fs;`
- [x] `helpers/copy.rs` - `use fs_err::{self as fs, File};`
- [x] `helpers/file.rs` - `use fs_err as fs;`
- [x] `helpers/hash.rs` - `use fs_err as fs;` and `use fs_err::File;`
- [x] `helpers/ignore.rs` - `use fs_err::{self as fs, OpenOptions};`
- [x] `helpers/layout.rs` - `use fs_err as fs;`
- [x] `helpers/store.rs` - `use fs_err as fs;`
- [x] `helpers/backend.rs` - `use fs_err as fs;` (in tests)
- [x] `ops/init.rs` - `use fs_err as fs;`
- [x] `ops/add.rs` - `use fs_err as fs;`
- [x] `ops/get.rs` - `use fs_err as fs;` (in tests)
- [x] `ops/status.rs` - `use fs_err as fs;` (in tests)
- [x] `ops/materialize.rs` - `use fs_err as fs;`
- [x] `types/config.rs` - `use fs_err as fs;`
- [x] `types/manifest.rs` - `use fs_err as fs;`
- [x] `types/metadata.rs` - `use fs_err as fs;`

### Justfile lint rule

- [x] Added `check-std-fs` recipe with PCRE2 negative lookahead
- [x] Allows `std::fs::Permissions` and `std::fs::Metadata` (types fs-err doesn't re-export)
- [x] Pattern: `'std::fs(?!::(Permissions|Metadata)\b)'`

### Summary

- **122 tests passing** (116 dvs-core + 6 dvs-cli)
- All filesystem I/O uses fs_err for better error messages
- Lint rule enforces consistent usage
- `std::fs::Permissions` allowed (required for Unix permission setting)

## Plan 025: Hashing + Fingerprinting for DVS (Phase 1)

Implemented multi-algorithm hashing support with BLAKE3 (default), XXH3 (fast non-cryptographic), and SHA-256 (HTTP interoperability).

### Dependencies (optional features)

- [x] Added `xxhash-rust = { version = "0.8", features = ["xxh3"] }` to workspace
- [x] Added `sha2 = "0.10"` to workspace
- [x] dvs-core features: `blake3` (default), `xxh3`, `sha256`, `all-hashes`

### helpers/hash.rs - Multi-algorithm hashing

- [x] `Hasher` trait with `update()`, `finalize()`, `algorithm()` methods
- [x] `Blake3Hasher` - BLAKE3 streaming hasher (feature: `blake3`)
- [x] `Xxh3Hasher` - XXH3 streaming hasher (feature: `xxh3`)
- [x] `Sha256Hasher` - SHA-256 streaming hasher (feature: `sha256`)
- [x] `hash_blake3()`, `hash_xxh3()`, `hash_sha256()` - Single-shot hash functions
- [x] `create_hasher(algo)` - Factory function for hasher by algorithm
- [x] `default_algorithm()` - Returns default based on enabled features (BLAKE3 > XXH3 > SHA-256)
- [x] `get_file_hash_with_algo(path, algo)` - Hash file with specified algorithm
- [x] `hash_bytes(data, algo)` - Hash bytes with specified algorithm
- [x] `verify_hash_with_algo(path, expected, algo)` - Verify file hash with algorithm

### types/oid.rs - XXH3 support

- [x] Added `Oid::xxh3(hex)` constructor
- [x] Added `Serialize`/`Deserialize` derives to `HashAlgo`
- [x] `#[serde(rename_all = "lowercase")]` for clean JSON output

### types/config.rs - Configurable hash algorithm

- [x] Added `hash_algo: Option<HashAlgo>` field (defaults to Blake3 for backward compatibility)
- [x] `Config::with_hash_algo()` constructor for specifying algorithm
- [x] `Config::hash_algorithm()` - Returns configured or default algorithm
- [x] `#[serde(skip_serializing_if = "Option::is_none")]` to keep files clean

### types/metadata.rs - Hash algorithm tracking

- [x] Added `hash_algo: HashAlgo` field with default to Blake3
- [x] `Metadata::with_algo()` constructor for specifying algorithm
- [x] `Metadata::checksum()` - Accessor for the checksum field
- [x] Backward compatible: old metadata files default to Blake3

### ops/add.rs - Configurable algorithm in add

- [x] Uses `config.hash_algorithm()` for file hashing
- [x] Creates metadata with configured algorithm via `Metadata::with_algo()`
- [x] Comparison checks both checksum and algorithm match

### Tests - 4 new algorithm tests

- [x] `test_hash_small_file_xxh3` - XXH3 file hashing
- [x] `test_hash_small_file_sha256` - SHA-256 file hashing
- [x] `test_xxh3_hasher_streaming` - XXH3 streaming
- [x] `test_sha256_hasher_streaming` - SHA-256 streaming

### Summary

- **123 tests passing** with `all-hashes` feature (119 default + 4 algorithm tests)
- Multi-algorithm support: BLAKE3 (64 char), XXH3 (16 char), SHA-256 (64 char)
- Feature flags for optional algorithm dependencies
- Backward compatible with existing metadata files
- Phase 2 (chunking, Merkle trees) and Phase 3 (tables, sketches) deferred
