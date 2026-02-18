# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

dvs-rpkg is an R package providing bindings for DVS (Data Version Control System) — a system for versioning large binary/text files using content-addressable storage with blake3 hashing. The R package wraps a Rust core library (`../dvs`) via the miniextendr FFI framework.

## Build & Development Commands

```bash
# Configure (required before first build, re-run after changing configure.ac)
./configure                    # CRAN mode (offline, vendored deps)
NOT_CRAN=true ./configure      # Dev mode (network access, locked deps)

# Build and install the R package
R CMD build .
R CMD INSTALL .

# Dev workflow (in R console) — bootstrap.R auto-sets NOT_CRAN=true
devtools::load_all()           # Build Rust + load package
devtools::document()           # Regenerate roxygen docs

# Build just the Rust library
cargo build --manifest-path src/rust/Cargo.toml --target-dir rust-target

# Run R tests
devtools::test()               # All tests via testthat
testthat::test_file("tests/testthat/test-foo.R")  # Single test file
```

## Architecture

### Rust → R Binding Pipeline

1. **Rust core** (`../dvs/`) — standalone crate with init, add, get, status operations
2. **FFI layer** (`src/rust/lib.rs`) — 4 exported functions using `#[miniextendr]` proc-macro:
   - `dvs_init`, `dvs_add`, `dvs_status`, `dvs_get`
3. **R wrapper generation** (`src/rust/document.rs`) — binary that parses miniextendr output to auto-generate `R/dvs-wrappers.R`
4. **C entrypoint** (`src/entrypoint.c.in`) — R package init, registers `.Call()` bindings

`R/dvs-wrappers.R` is **auto-generated** — do not edit manually. Changes to the R API surface are made in `src/rust/lib.rs`.

### Build System

The build uses autoconf (`configure.ac`) + Cargo with two modes:

- **CRAN mode** (`NOT_CRAN=false`): offline, uses vendored deps from `inst/vendor.tar.xz`
- **Dev mode** (`NOT_CRAN=true`): network-enabled, uses `--locked` flag

`Makevars.in` is a template — `configure` generates `src/Makevars`. The build compiles Rust to a static library, then links it into the R shared object.

### Key Dependencies

- **miniextendr** (git: CGMossa/miniextendr) — Rust-to-R FFI framework with serde support
- **dvs** (path: `../dvs`) — core DVS library (patched in Cargo.toml)
- **R imports**: jsonlite
- **Rust requires**: rustc >= 1.85.0

### Vendoring

Rust dependencies are vendored via `cargo vendor` during configure. For CRAN distribution, they're compressed into `inst/vendor.tar.xz`. The `vendor/` directory is auto-generated — don't commit manual changes to it.

## Key Files

| File | Purpose |
|------|---------|
| `src/rust/lib.rs` | All exported Rust functions (the API surface) |
| `src/rust/Cargo.toml` | Rust deps, patches dvs crate to `../../../dvs` |
| `R/dvs-wrappers.R` | Auto-generated R wrappers (do not edit) |
| `configure.ac` | Autoconf build config (~700 lines) |
| `src/Makevars.in` | Build recipe template |
| `bootstrap.R` | Pre-configure script for devtools workflows |
| `DESCRIPTION` | R package metadata |

## Environment Variables

| Variable | Effect |
|----------|--------|
| `NOT_CRAN` | `true` = dev mode (network), `false` = CRAN mode (offline) |
| `CARGO_PROFILE` | `release` (default) or `debug` |
| `DVS_FEATURES` | Comma-separated cargo features (e.g., `nonapi`) |
| `RUST_TOOLCHAIN` | e.g., `+stable`, `+nightly` |
