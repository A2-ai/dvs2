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

## CLI Installation

The CLI binary is named `dvs` (not `dvs-cli`). Install from source:

```bash
# Install with locked dependencies (recommended)
cargo install --path dvs-cli --locked

# Force reinstall if already installed
cargo install --path dvs-cli --locked --force

# Or via cargo directly from the workspace
cargo build -p dvs-cli --release
# Binary will be at target/release/dvs
```

After installation, the `dvs` command is available:

```bash
dvs init <storage_dir>
dvs add <files...>
dvs get <files...>
dvs status [files...]
dvs push [--remote URL]
dvs pull [--remote URL]
dvs materialize [files...]
dvs log [-n N]
dvs rollback <target>
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
just rpkg-vendor-force  # Force re-vendor after miniextendr changes
just rpkg-vendor-detect # Re-vendor if miniextendr sources changed

# Combined
just build-all          # Build everything
just test-all           # Test everything
```

### miniextendr Vendoring

The R package vendors miniextendr crates for CRAN compliance. When you modify miniextendr sources at `/Users/elea/Documents/GitHub/miniextendr/`, use the vendor recipes to update:

**After modifying miniextendr sources:**

```bash
# Automatic staleness detection + copy from source (recommended)
just rpkg-vendor-detect

# Force re-vendor (always updates, even if unchanged)
just rpkg-vendor-force

# Custom miniextendr path
just rpkg-vendor-with-staleness /path/to/miniextendr
```

**What happens:**

1. Checks if miniextendr sources are newer than vendor stamp
2. Copies miniextendr-api, miniextendr-macros, miniextendr-lint from source
3. Patches Cargo.toml files to remove workspace inheritance
4. Runs `cargo vendor` for other transitive dependencies
5. Creates vendor.tar.xz for CRAN builds

**Environment variables:**

- `FORCE_VENDOR=true` - Force re-vendor even if stamp file exists
- `MINIEXTENDR_SOURCE_DIR=/path/to/miniextendr` - Path to miniextendr source for copying and staleness detection

## Related Repositories

- `/Users/elea/Documents/GitHub/miniextendr` - miniextendr source (R-Rust interop framework)
- `/Users/elea/Documents/a2ai_github/dvs` - Reference implementation

## Issues for Upstream (miniextendr)

See `/Users/elea/Documents/GitHub/miniextendr/dvs_review/case-sensitivity-issue.md` for the case sensitivity fix that should be upstreamed to miniextendr's rpkg template.

## Plans

Once a plan is executed, please store a summary of the plan, underneath of
a heading in DONE.md. Also update the plan status in TODO.md.

## Development Guidelines

- **No backward compatibility**: We do not care about backward compatibility for now. Feel free to make breaking changes to APIs, file formats, and configurations as needed.
- **DVS naming**: Always use "DVS" or "dvs" (Data Version System). Never use "DVC" which is a different project.
- **Use `fs_err` instead of `std::fs`**: All filesystem operations in dvs-core and dvs-cli must use `fs_err` instead of `std::fs` for better error messages. The only exceptions are `std::fs::Permissions` and `std::fs::Metadata` types (which `fs_err` doesn't re-export). This is enforced by `just check-std-fs` lint.
