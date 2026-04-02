# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

dvs-rpkg is an R package providing bindings for DVS (Data Version Control System) — a system for versioning large binary/text files using content-addressable storage with blake3 hashing. The R package wraps a Rust core library (`../dvs`) via the miniextendr FFI framework.

## Principles

- **Edit `.in` templates, not generated files**: `src/Makevars` is generated from `src/Makevars.in`. `src/rust/.cargo/config.toml` is generated from `src/rust/cargo-config.toml.in`. `configure` is generated from `configure.ac`. Always edit the `.in` source.
- **`R/dvs-wrappers.R` is auto-generated**: Never edit manually. Changes to the R API surface are made in `src/rust/lib.rs`.
- **Don't over-engineer**: `#[allow(clippy::too_many_arguments)]` is preferred over refactoring to options structs when not needed.
- **Use `#[cfg(feature = "...")]` attribute gating**, not `cfg!()` runtime checks.
- **Fix issues when encountered**: Don't document warnings as "known issues" — fix them.

## Build & Development Commands

### Quick Reference (justfile)

Run from the repo root (`dvs2/`). The justfile sets `NOT_CRAN=true` automatically.

```bash
# Configure (required before first R CMD operation)
just rpkg-configure          # Runs autoconf + bash ./configure

# Rust-only development (fast iteration, no R required)
just rpkg-build              # cargo build
just rpkg-check              # cargo check
just rpkg-clippy             # cargo clippy
just rpkg-test               # cargo test + R tests
just rpkg-fmt                # cargo fmt

# R package development
just rpkg-install            # R CMD INSTALL (compiles Rust + generates R wrappers)
just rpkg-document           # devtools::document() (regenerates NAMESPACE + man pages)

# Combined targets
just build-all               # Build dvs + dvs-rpkg
just test-all                # Test dvs + dvs-rpkg
just clippy-all              # Clippy dvs + dvs-rpkg
just fmt-all                 # Format dvs + dvs-rpkg
just ci                      # Full CI: fmt-check, clippy, check-std-fs, test
```

### Manual Commands (from dvs-rpkg/)

```bash
# Configure (required before first build, re-run after changing configure.ac)
bash ./configure                    # Dev mode (auto-detects monorepo)
NOT_CRAN=false bash ./configure     # CRAN mode (offline, vendored deps)

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

### Critical: Configure Before R CMD Operations

Always run `./configure` (or `just rpkg-configure`) before any R CMD operation. Always invoke configure with `bash` (e.g., `bash ./configure`, not `./configure`). The autoconf-generated shebang is `#!/bin/sh`, which on some systems causes spurious errors.

## Architecture

### Data Flow

1. **Rust core** (`../dvs/`) — standalone crate: init, add, get, status operations
2. **FFI layer** (`src/rust/lib.rs`) — exported functions using `#[miniextendr]` proc-macro
3. **cdylib build** — compiled with linkme `#[distributed_slice]` entries for R API discovery
4. **R wrapper generation** — cdylib is loaded in R, `miniextendr_write_wrappers()` auto-generates `R/dvs-wrappers.R`
5. **Static library link** — Rust staticlib + C stub → `dvs.so`/`dvs.dll`
6. **R calls** — user calls R functions → `.Call()` → C entry points → Rust functions

### Build System

The build uses autoconf (`configure.ac`) + Cargo with two modes:

| Mode | Trigger | Behavior |
|------|---------|----------|
| **Dev** | `NOT_CRAN=true` or monorepo detected | Network-enabled, uses `[patch]` paths to `../dvs` |
| **CRAN** | `NOT_CRAN=false` or vendor artifacts present | Offline, unpacks `inst/vendor.tar.xz` |

`Makevars.in` is a template — `configure` generates `src/Makevars`. The build produces two libraries:
- **staticlib** — linked into the final R shared object
- **cdylib** — temporary, used only for R wrapper generation (deleted after)

### Key Dependencies

- **miniextendr** (git: CGMossa/miniextendr) — Rust-to-R FFI framework with serde support
- **dvs** (path: `../dvs`) — core DVS library (patched in Cargo.toml)
- **R imports**: jsonlite
- **Rust requires**: rustc >= 1.85.0

### Vendoring

Rust dependencies are vendored via `cargo vendor` during configure. For CRAN distribution, they're compressed into `inst/vendor.tar.xz`. The `vendor/` directory is auto-generated — don't commit manual changes to it.

The `tools/vendor-crates.R` script handles packing vendored dependencies.

## Key Files

| File | Purpose |
|------|---------|
| `src/rust/lib.rs` | All exported Rust functions (the API surface) |
| `src/rust/Cargo.toml` | Rust deps, patches dvs crate to `../../../dvs` |
| `R/dvs-wrappers.R` | Auto-generated R wrappers (**do not edit**) |
| `R/dvs-package.R` | Package declaration, `@useDynLib dvs` |
| `configure.ac` | Autoconf build config (~477 lines) |
| `src/Makevars.in` | Build recipe template (~131 lines) |
| `src/rust/cargo-config.toml.in` | Cargo config template for offline/CRAN builds |
| `bootstrap.R` | Pre-configure script for devtools workflows |
| `DESCRIPTION` | R package metadata |
| `NAMESPACE` | R exports (regenerated by `devtools::document()`) |
| `tools/vendor-crates.R` | Packs vendored Rust dependencies |
| `src/stub.c` | C linker stub |

## Exported R Functions

All defined in `src/rust/lib.rs` with `#[miniextendr]`:

| Function | Purpose | Returns |
|----------|---------|---------|
| `dvs_init(storage_path, root_dir?, group?, metadata_folder_name?, no_compression?)` | Initialize a DVS repository | List |
| `dvs_add(files?, message?, glob?, dry_run?)` | Add files to content-addressable storage | DataFrame |
| `dvs_status(current?, absent?, unsynced?)` | Report sync status of managed files | DataFrame |
| `dvs_get(files?, glob?, dry_run?)` | Retrieve files from storage | DataFrame |

Results are returned as DataFrames using miniextendr's `AsSerializeRow` trait. Errors propagate as `anyhow::Result<T>` converted to R error objects.

## Environment Variables

| Variable | Effect |
|----------|--------|
| `NOT_CRAN` | `true` = dev mode (network), `false`/unset = CRAN mode (offline) |
| `CARGO_PROFILE` | `release` (default) or `debug` |
| `DVS_FEATURES` | Comma-separated cargo features (e.g., `nonapi`) |
| `RUST_TOOLCHAIN` | e.g., `+stable`, `+nightly` |
| `PREPARE_CRAN` | `true` = explicit CRAN release prep mode |
| `FORCE_VENDOR` | `1` = force re-vendor in dev mode |

## Adding New Rust Functions

1. Add `#[miniextendr]` function to `src/rust/lib.rs` (or a new `.rs` file reachable via `mod` from `lib.rs`)
2. Function must be `pub` (or `pub(crate)` with `r_name` attribute) to get `@export` in R wrappers
3. No module declaration needed — functions self-register via linkme's `#[distributed_slice]`
4. Rebuild:

```bash
just rpkg-configure       # 1. Configure build
just rpkg-install         # 2. Build + install (compiles Rust, generates R wrappers)
just rpkg-document        # 3. Regenerate NAMESPACE via roxygen2
```

R wrappers are auto-generated during build. The build compiles a cdylib, loads it in R, calls `miniextendr_write_wrappers()`, then deletes the cdylib.

## Testing

```bash
just rpkg-test            # Rust unit tests + R tests
devtools::test()          # All R tests via testthat (from R console)
```

Tests live in `tests/testthat/`. The package uses testthat edition 3.

## Capturing Command Output

Always redirect long-running R/Cargo command output to a log file, then read the log. This ensures full output without truncation.

```bash
just rpkg-install 2>&1 > /tmp/rpkg-install.log
just rpkg-document 2>&1 > /tmp/rpkg-document.log
just rpkg-test 2>&1 > /tmp/rpkg-test.log
```

After the command finishes, use the Read tool to read the log file. Do NOT use `tail` or `head`.

## Common Issues

### "configure: command not found"

Run autoconf first, or use the justfile which handles this:

```bash
cd dvs-rpkg && autoconf && bash ./configure
# or simply:
just rpkg-configure
```

### Stale R wrappers after Rust changes

R wrappers are auto-generated during build. Just rebuild:

```bash
just rpkg-configure && just rpkg-install && just rpkg-document
```

### R tests fail with "could not find function"

Check:
1. Function has `#[miniextendr]`
2. Function is `pub`
3. Module is reachable via `mod` declarations from `lib.rs`
4. NAMESPACE is stale — run `just rpkg-document`

### Sandbox restrictions

Claude Code's sandbox blocks compilation. Commands that compile code (`just rpkg-install`, `R CMD INSTALL`, `cargo build`) require `dangerouslyDisableSandbox: true`.

### macOS `/tmp` symlink

macOS symlinks `/tmp` → `/private/tmp`. This causes `canonicalize()` vs `starts_with()` mismatches in tests. Use `canonicalize()` on both sides of path comparisons.
