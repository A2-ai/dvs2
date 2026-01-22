# `dvs`

Rewrite of `dvs`, the data-version-control system made by A2-AI.

DVS (Data Version System) is a tool for versioning large or sensitive files under Git without tracking the file content directly. It uses content-addressable storage with blake3 hashing.

## Installation

The CLI binary is named `dvs`. Install from source:

```bash
# Install with locked dependencies (recommended)
cargo install --path dvs-cli --locked

# Force reinstall if already installed
cargo install --path dvs-cli --locked --force
```

Or build directly:

```bash
cargo build -p dvs-cli --release
# Binary will be at target/release/dvs
```

## Usage

```bash
# Initialize DVS in a repository
dvs init <storage_dir>

# Add files to DVS tracking
dvs add <files...>

# Restore files from storage
dvs get <files...>

# Check file status
dvs status [files...]

# Push objects to remote
dvs push [--remote URL]

# Pull objects from remote
dvs pull [--remote URL]

# Materialize files from manifest
dvs materialize [files...]

# View reflog history
dvs log [-n N]

# Rollback to previous state
dvs rollback <target>
```

### Batch Operations

Commands that accept file arguments also support `--batch` to read paths from stdin:

```bash
# Add files listed in a file
cat files.txt | dvs add --batch

# Process output from find
find . -name "*.csv" | dvs add --batch

# Batch format supports comments and blank lines
echo "data.csv
# This is a comment
results.json" | dvs add --batch
```

### Output Formats

All commands support `--format json` for machine-readable output:

```bash
dvs status --format json
dvs add data.csv --format json
```

Use `--quiet` to suppress non-error output, or `--output null` to discard output entirely.

## Development

### Building

```bash
# Build workspace (dvs-core, dvs-cli)
just build

# Build R package
just rpkg-build

# Build everything
just build-all
```

### Testing

```bash
# Run workspace tests
just test

# Run R package Rust tests
just rpkg-test

# Run all tests
just test-all
```

### R Package Maintenance

The R package (`dvsR`) uses vendored miniextendr crates for CRAN compliance. When developing with a local miniextendr checkout, use these commands to keep vendored sources up to date:

```bash
# Automatic staleness detection (recommended)
# Re-vendors only if miniextendr sources have changed
just rpkg-vendor-detect

# Force re-vendor (always updates vendored crates)
just rpkg-vendor-force

# Custom miniextendr path
just rpkg-vendor-with-staleness /path/to/miniextendr

# Configure R package (generates Cargo.toml, Makevars, etc.)
just rpkg-configure

# Install R package
just rpkg-install
```

### Code Quality

```bash
# Format code
just fmt

# Run clippy
just clippy

# Run all CI checks
just ci
```
