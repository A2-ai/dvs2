# DVS Project - Claude Context

## Project Overview

DVS (Data Version System) is a tool for versioning large or sensitive files under Git without tracking the file directly. It uses content-addressable storage with blake3 hashing.

## Repository Structure

```shell
dvsexperimental/
├── Cargo.toml              # Workspace manifest (dvs-core, dvs-cli)
├── dvs-core/               # Core Rust library
├── dvs-cli/                # CLI application
├── dvsR/                   # R package with Rust bindings (miniextendr)
│   ├── DESCRIPTION
│   ├── NAMESPACE
│   ├── R/                  # R wrapper functions
│   ├── src/
│   │   ├── rust/           # Embedded Rust crate
│   │   │   ├── Cargo.toml.in   # Template (configure generates Cargo.toml)
│   │   │   ├── lib.rs          # Rust functions with #[miniextendr]
│   │   │   └── document.rs.in  # R wrapper generator template
│   │   ├── vendor/         # Vendored miniextendr crates
│   │   ├── entrypoint.c.in # C entry point template
│   │   ├── mx_abi.c.in     # Trait ABI template
│   │   └── Makevars.in     # Build rules template
│   ├── configure.ac        # Autoconf configuration
│   └── configure           # Generated configure script
└── justfile                # Build recipes
```

## Key Technical Details

### Workspace vs R Package

- **Root workspace** (`Cargo.toml`): Contains `dvs-core` and `dvs-cli`
- **R package** (`dvsR/src/rust/`): Standalone Rust crate, NOT part of workspace
  - Uses vendored miniextendr crates in `src/vendor/`
  - Cargo.toml is generated from Cargo.toml.in by configure

### Case Sensitivity Issue (Important!)

R package names can be mixed-case (e.g., `dvsR`) but autoconf's `PACKAGE_TARNAME` is always lowercase (`dvsr`). This caused symbol registration to fail.

**Solution**: Use `@PACKAGE_NAME@` (preserves case) for R-facing symbols:
- `R_init_@PACKAGE_NAME@()` in entrypoint.c.in
- `miniextendr_set_altrep_pkg_name("@PACKAGE_NAME@")`

Use `@PACKAGE_TARNAME_RS@` (lowercase) for Rust module names.

### miniextendr Integration

The R package uses miniextendr for Rust-R interop:
- `#[miniextendr]` attribute on Rust functions
- `miniextendr_module!` macro to register functions
- Vendored crates require edition 2024 for miniextendr-macros and miniextendr-lint

### Build Process

1. `./configure` (or `just rpkg-configure`) generates:
   - `src/rust/Cargo.toml` from `Cargo.toml.in`
   - `src/Makevars` from `Makevars.in`
   - `src/entrypoint.c` from `entrypoint.c.in`
   - `src/mx_abi.c` from `mx_abi.c.in`

2. R CMD INSTALL builds:
   - Compiles Rust to static library (`libdvsr.a`)
   - Runs `document` binary to generate R wrappers
   - Links C and Rust into `dvsR.so`

## Useful Commands

```bash
# Workspace
just build              # Build dvs-core, dvs-cli
just test               # Run tests
just check              # Check without building

# R Package
just rpkg-configure     # Run configure
just rpkg-build         # Build Rust library
just rpkg-install       # Install R package
just rpkg-document      # Generate R wrappers
just rpkg-clean         # Clean build artifacts

# Combined
just build-all          # Build everything
just test-all           # Test everything
```

## Related Repositories

- `/Users/elea/Documents/GitHub/miniextendr` - miniextendr source (R-Rust interop framework)
- `/Users/elea/Documents/a2ai_github/dvs` - Reference implementation

## Issues for Upstream (miniextendr)

See `/Users/elea/Documents/GitHub/miniextendr/dvs_review/case-sensitivity-issue.md` for the case sensitivity fix that should be upstreamed to miniextendr's rpkg template.

## Plans

Once a plan is executed, please store a summary of the plan, underneath of
a heading in DONE.md.
