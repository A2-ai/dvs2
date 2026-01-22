# DVS Architecture

DVS is organized as a Rust workspace with four crates plus an R package:

```text
┌─────────────────────────────────────────────────────────────────────┐
│                         Client Layer                                 │
├─────────────┬─────────────┬─────────────────────────────────────────┤
│    dvsR     │   dvs-cli   │            dvs-testkit                  │
│ (R Package) │   (CLI)     │         (Test Harness)                  │
└──────┬──────┴──────┬──────┴──────────────┬──────────────────────────┘
       │             │                      │
       └─────────────┴──────────────────────┘
                              │
┌─────────────────────────────┴───────────────────────────────────────┐
│                         dvs-core                                     │
│                    (Pure Business Logic)                             │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                        Types Layer                            │   │
│  │  Config, Metadata, Manifest, Oid, ReflogEntry, DvsError      │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      Operations Layer                         │   │
│  │  init, add, get, status, push, pull, materialize, log,       │   │
│  │  rollback, merge_repo                                         │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                       Helpers Layer                           │   │
│  │  hash, copy, file, config, backend, git_ops, store, layout,  │   │
│  │  ignore, reflog, version                                      │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Crates

### dvs-core

Pure Rust library containing all business logic. No CLI parsing, no FFI.

- **types/**: Config, Metadata, Manifest, Oid, ReflogEntry, DvsError
- **ops/**: init, add, get, status, push, pull, materialize, log, rollback, merge_repo
- **helpers/**: hash, copy, file, config, backend, git_ops, store, layout, ignore, reflog

### dvs-cli

Command-line interface using clap. Thin wrapper over dvs-core.

### dvs-testkit

Testing utilities: TestRepo, WorkspaceSnapshot, InterfaceRunner trait with CoreRunner/CliRunner/RRunner.

### dvsR

R package using miniextendr for FFI. Communicates with dvs-core via JSON.

## Feature Flags (dvs-core)

```toml
[features]
default = ["blake3", "mmap", "walkdir", "toml-config", "serde"]
blake3 = ["dep:blake3"]          # BLAKE3 hashing (default)
sha256 = ["dep:sha2"]            # SHA-256 hashing
xxh3 = ["dep:xxhash-rust"]       # XXH3 fast hashing
mmap = ["dep:memmap2"]           # Memory-mapped file hashing
walkdir = ["dep:walkdir"]        # Recursive directory listing
yaml-config = ["dep:serde_yaml"] # YAML config file support
toml-config = ["dep:toml"]       # TOML config file support
```

Git operations use the system `git` CLI.

## Error Handling

All crates use `exn`-based errors with stable `error_type()` strings for R interop:

```rust
pub type DvsError = Exn<ErrorKind>;

impl ErrorKind {
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::FileNotFound => "file_not_found",
            // ...
        }
    }
}
```
