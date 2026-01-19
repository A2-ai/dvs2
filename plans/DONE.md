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
