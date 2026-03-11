# CLAUDE.md

## Project overview

dvs (Data Version Control System) versions large or sensitive files under Git without tracking file contents directly. It uses content-addressable storage with blake3 hashing and optional zstd compression.

### Workspace layout

| Path | What |
|------|------|
| `dvs/` | Core Rust library (hashing, storage backends, metadata) |
| `dvs-cli/` | CLI binary (`dvs init`, `dvs add`, `dvs get`, `dvs status`) |
| `dvs-rpkg/` | R package wrapping the core via miniextendr FFI |

The root `Cargo.toml` workspace contains `dvs` and `dvs-cli`. The R package has its own Cargo workspace at `dvs-rpkg/src/rust/Cargo.toml`.

## Build & verify

Always run fmt, clippy, and test before considering work done:

```bash
# Format
cargo fmt                  # workspace
just rpkg-fmt              # R package Rust code

# Lint
cargo clippy --workspace --all-targets
just rpkg-clippy

# Test
cargo test --workspace
just rpkg-test

# Quick all-in-one
just ci                    # fmt-check + clippy + check-std-fs + test (workspace only)
```

### R package (dvs-rpkg)

The R package must also build and install cleanly. After touching `dvs/` or `dvs-rpkg/src/rust/`:

```bash
just rpkg-build            # cargo build for the R crate
just rpkg-install          # full R CMD INSTALL
```

See `dvs-rpkg/CLAUDE.md` for detailed R package architecture and build system docs.

## Commit rules

- **Always commit `Cargo.lock`** (both root and `dvs-rpkg/src/rust/Cargo.lock`). The project depends on git-sourced crates (miniextendr) so lockfiles are essential for reproducibility.
- Use `fs_err` instead of `std::fs` in the `dvs` crate. The `just check-std-fs` target enforces this.

## Updating miniextendr

miniextendr is a git dependency from CGMossa/miniextendr. To pull the latest:

```bash
just rpkg-update
```

This runs `cargo update` on the R package's Cargo.toml. After updating, the R package scaffolding may also need updating via the `minirextendr::upgrade_miniextendr_package()` R function.

## Development iteration (R bindings)

`R/dvs-wrappers.R` is **auto-generated** from `dvs-rpkg/src/rust/lib.rs` -- do not edit it manually. Any invocation that rebuilds the R package (e.g. `devtools::load_all("dvs-rpkg")`) regenerates the wrappers.

## Policy: fix issues when you see them

When tests fail or issues are encountered during work on a branch, fix them immediately rather than leaving them as "pre-existing". Backpropagate fixes to affected branches (cherry-pick) and propose PRs against main when the fix applies there too. Do not push directly to main — always use a PR.

## Key conventions

- Edition 2024, rust-version 1.85
- Verbose output goes to stderr via `eprintln!`
- Parallel file processing uses rayon; verbose messages are prefixed with `[filename]`
- `OutputOptions { dry_run, verbosity, timing_tx }` is the shared options struct across all commands
- `AddOptions` wraps `OutputOptions` with add-specific fields (message, compression)
