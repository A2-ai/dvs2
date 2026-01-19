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
