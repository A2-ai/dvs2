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
