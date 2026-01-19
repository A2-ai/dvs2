# Plan 001: Decoupling Architecture

## Overview

This plan decouples the DVS codebase into independent layers:

1. **dvs-core**: Pure Rust library (no I/O bindings, just business logic)
2. **dvsR**: R package bindings (uses miniextendr)
3. **dvs-cli**: Command-line interface
4. **dvs-daemon**: Local RESTful API daemon (single-user)
5. **dvs-server**: Multi-user server with authentication (future)

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Client Layer                                 │
├─────────────┬─────────────┬─────────────┬─────────────────────────-─┤
│    dvsR     │   dvs-cli   │ dvs-daemon  │      dvs-server           │
│ (R Package) │   (CLI)     │ (REST API)  │   (Multi-user Server)     │
│             │             │             │                           │
│ miniextendr │   clap      │   axum      │   axum + auth             │
└──────┬──────┴──────┬──────┴──────┬──────┴───────────┬───────────────┘
       │             │             │                   │
       └─────────────┴─────────────┴───────────────────┘
                              │
┌─────────────────────────────┴───────────────────────────────────────┐
│                         dvs-core                                     │
│                    (Pure Business Logic)                             │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                        Types Layer                            │   │
│  │  Config, Metadata, FileInfo, Status, Outcome, Error          │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      Operations Layer                         │   │
│  │  init(), add(), get(), status(), info()                      │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                       Helpers Layer                           │   │
│  │  hash, copy, file, config, parse, repo, ignore, cache        │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Crate Responsibilities

### dvs-core (Pure Library)

**No dependencies on**: miniextendr, clap, axum, tokio

**Responsibilities**:
- Type definitions (Config, Metadata, FileInfo, etc.)
- Core operations (init, add, get, status)
- Helper utilities (hashing, file ops, config parsing)
- Error types

**Public API**:
```rust
// Types
pub struct Config { ... }
pub struct Metadata { ... }
pub struct FileInfo { ... }
pub struct AddResult { ... }
pub struct GetResult { ... }
pub struct StatusResult { ... }
pub enum Outcome { Copied, Present, Error }
pub enum FileStatus { Current, Absent, Unsynced, Error }

// Operations
pub fn init(storage_dir: &Path, permissions: Option<u32>, group: Option<&str>) -> Result<Config>;
pub fn add(files: &[PathBuf], message: Option<&str>) -> Result<Vec<AddResult>>;
pub fn get(files: &[PathBuf]) -> Result<Vec<GetResult>>;
pub fn status(files: &[PathBuf]) -> Result<Vec<StatusResult>>;

// Helpers (internal, but may expose some)
mod helpers {
    pub mod hash;
    pub mod copy;
    pub mod file;
    pub mod config;
    pub mod parse;
    pub mod repo;
    pub mod ignore;
    pub mod cache;
}
```

### dvsR (R Package)

**Dependencies**: dvs-core, miniextendr-api

**Responsibilities**:
- FFI bindings via miniextendr
- R data frame conversions
- Error handling for R

**Public API** (R functions):
```r
dvs_init(storage_directory, permissions = NULL, group = NULL)
dvs_add(files, message = NULL, split_output = FALSE)
dvs_get(files, split_output = FALSE)
dvs_status(files = "", split_output = FALSE)
```

### dvs-cli (Command Line)

**Dependencies**: dvs-core, clap

**Responsibilities**:
- Parse command-line arguments
- Pretty-print output
- Exit codes

**Commands**:
```bash
dvs init <storage_dir> [--permissions <octal>] [--group <name>]
dvs add <files...> [--message <msg>]
dvs get <files...>
dvs status [files...]
```

### dvs-daemon (Local REST API)

**Dependencies**: dvs-core, axum, tokio

**Responsibilities**:
- HTTP server on localhost
- JSON API
- Single-user (no auth)

**Endpoints**:
```
POST /init          { storage_dir, permissions?, group? }
POST /add           { files, message? }
POST /get           { files }
GET  /status        ?files=...
GET  /health
```

### dvs-server (Multi-user Server) - Future

**Dependencies**: dvs-core, axum, tokio, tower, sqlx

**Responsibilities**:
- Multi-tenant storage
- Authentication/authorization
- Project management
- Remote storage backends (S3, etc.)

## Module Structure

```
dvsexperimental/
├── Cargo.toml              # Workspace
├── dvs-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          # Public API
│       ├── types/
│       │   ├── mod.rs
│       │   ├── config.rs
│       │   ├── metadata.rs
│       │   ├── file_info.rs
│       │   ├── outcome.rs
│       │   └── error.rs
│       ├── ops/
│       │   ├── mod.rs
│       │   ├── init.rs
│       │   ├── add.rs
│       │   ├── get.rs
│       │   └── status.rs
│       └── helpers/
│           ├── mod.rs
│           ├── hash.rs
│           ├── copy.rs
│           ├── file.rs
│           ├── config.rs
│           ├── parse.rs
│           ├── repo.rs
│           ├── ignore.rs
│           └── cache.rs
├── dvs-cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── dvs-daemon/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── routes.rs
├── dvs-server/             # Future
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── dvsR/                   # R package (separate build)
    └── src/rust/
        └── lib.rs
```

## Implementation Order

1. **Phase 1**: dvs-core types and helpers (scaffolding with `todo!()`)
2. **Phase 2**: dvs-core operations
3. **Phase 3**: dvsR bindings (already started)
4. **Phase 4**: dvs-cli
5. **Phase 5**: dvs-daemon
6. **Phase 6**: dvs-server (future)

## Error Handling Strategy

All crates use a common error type from dvs-core:

```rust
#[derive(Debug, thiserror::Error)]
pub enum DvsError {
    #[error("not in a git repository")]
    NotInGitRepo,

    #[error("dvs.yaml not found - run dvs_init first")]
    NotInitialized,

    #[error("file not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("metadata not found: {path}")]
    MetadataNotFound { path: PathBuf },

    #[error("storage error: {message}")]
    StorageError { message: String },

    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch { path: PathBuf, expected: String, actual: String },

    #[error("permission denied: {message}")]
    PermissionDenied { message: String },

    #[error("config error: {message}")]
    ConfigError { message: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
```

## Feature Flags

dvs-core supports optional features:

```toml
[features]
default = []
rayon = ["dep:rayon"]      # Parallel hashing
cli = ["dep:clap"]         # CLI support (for dvs-cli)
```
