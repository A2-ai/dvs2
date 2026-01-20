# https://just.systems

# ============================================================================
# Path definitions
# ============================================================================

# Root workspace manifest
workspace_manifest := "Cargo.toml"

# R package embedded Rust crate
rpkg_dir := "dvsR"
rpkg_manifest := rpkg_dir / "src/rust/Cargo.toml"
rpkg_manifest_in := rpkg_dir / "src/rust/Cargo.toml.in"

# ============================================================================
# Default recipe
# ============================================================================

default:
    @just --list

# ============================================================================
# Workspace recipes (dvs-core, dvs-cli)
# ============================================================================

# Build the workspace (dvs-core, dvs-cli)
build *args:
    cargo build --manifest-path={{quote(workspace_manifest)}} --workspace {{args}}

# Build workspace in release mode
build-release *args:
    cargo build --manifest-path={{quote(workspace_manifest)}} --workspace --release {{args}}

# Run tests for workspace crates
test *args:
    cargo test --manifest-path={{quote(workspace_manifest)}} --workspace {{args}}

# Run clippy on workspace
clippy *args:
    cargo clippy --manifest-path={{quote(workspace_manifest)}} --workspace {{args}}

# Format workspace code
fmt *args:
    cargo fmt --manifest-path={{quote(workspace_manifest)}} {{args}}

# Check workspace without building
check *args:
    cargo check --manifest-path={{quote(workspace_manifest)}} --workspace {{args}}

# Check for std::fs usage in workspace Rust sources
# Allows std::fs::Permissions and std::fs::Metadata (types that fs-err doesn't re-export)
check-std-fs:
    @! rg -nP -g '*.rs' -e 'std::fs(?!::(Permissions|Metadata)\b)' -e 'std::\s*\{[^}]*\bfs\b[^}]*\}' dvs-core dvs-cli dvs-daemon dvs-server


# Run any cargo subcommand against the workspace
cargo subcmd *args:
    cargo {{subcmd}} --manifest-path={{quote(workspace_manifest)}} {{args}}

# Install the dvs CLI binary
install-cli *args:
    cargo install --force --locked --path=dvs-cli {{args}}

# ============================================================================
# R package recipes (dvsR)
# ============================================================================

# Configure the R package (generates Cargo.toml from .in template)
rpkg-configure:
    cd {{quote(rpkg_dir)}} && NOT_CRAN=true ./configure

# Vendor R package dependencies (runs configure with NOT_CRAN=true)
vendor:
    cd {{quote(rpkg_dir)}} && NOT_CRAN=true ./configure

# Build the R package Rust library
rpkg-build *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} --workspace {{args}}

# Build R package Rust library in release mode
rpkg-build-release *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} --workspace --release {{args}}

# Run tests for R package Rust code
rpkg-test *args:
    cargo test --manifest-path={{quote(rpkg_manifest)}} --workspace {{args}}

# Run clippy on R package Rust code
rpkg-clippy *args:
    cargo clippy --manifest-path={{quote(rpkg_manifest)}} --workspace {{args}}

# Check R package Rust code without building
rpkg-check *args:
    cargo check --manifest-path={{quote(rpkg_manifest)}} --workspace {{args}}

# Run any cargo subcommand against the R package manifest
rpkg-cargo subcmd *args:
    cargo {{subcmd}} --manifest-path={{quote(rpkg_manifest)}} {{args}}

# Generate R wrapper functions (runs the document binary)
rpkg-document:
    cargo run --manifest-path={{quote(rpkg_manifest)}} --bin document --release

# Install the R package
rpkg-install:
    NOT_CRAN=true Rscript -e 'install.packages("{{rpkg_dir}}", repos = NULL, type = "source")'

# Run devtools::document() on R package
rpkg-roxygen:
    Rscript -e 'devtools::document("{{rpkg_dir}}")'

# Run R CMD check on the package
rpkg-check-r *args:
    NOT_CRAN=true R CMD check {{rpkg_dir}} {{args}}

# Clean R package build artifacts
rpkg-clean:
    rm -rf {{rpkg_dir}}/src/rust/target
    rm -f {{rpkg_dir}}/src/*.o {{rpkg_dir}}/src/*.so {{rpkg_dir}}/src/*.dll
    rm -f {{rpkg_dir}}/src/Makevars {{rpkg_dir}}/src/entrypoint.c {{rpkg_dir}}/src/mx_abi.c
    rm -f {{rpkg_dir}}/src/rust/Cargo.toml {{rpkg_dir}}/src/rust/document.rs

# ============================================================================
# Combined recipes
# ============================================================================

# Build everything (workspace + R package)
build-all: build rpkg-build

# Build everything in release mode
build-all-release: build-release rpkg-build-release

# Test everything
test-all: test rpkg-test

# Check everything
check-all: check rpkg-check

# Format all Rust code
fmt-all: fmt
    cargo fmt --manifest-path={{quote(rpkg_manifest)}}

# Check formatting for all Rust code
fmt-check-all:
    cargo fmt --manifest-path={{quote(workspace_manifest)}} -- --check
    cargo fmt --manifest-path={{quote(rpkg_manifest)}} -- --check

# Alias fmt-check to fmt-check-all (default)
fmt-check: fmt-check-all

# Run clippy on everything
clippy-all: clippy rpkg-clippy

# ============================================================================
# Version management
# ============================================================================

# Sync or bump versions from DESCRIPTION.
# Examples:
#   just bump-version
#   just bump-version -- --bump=patch
#   just bump-version -- --bump=dev
#   just bump-version -- --bump=dev+
#   just bump-version -- --set=0.1.0.9000
#   just bump-version -- --mode=workspace
bump-version *args:
    Rscript tools/bump-version.R {{rpkg_dir}} {{args}}
