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
