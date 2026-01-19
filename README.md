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

## Developer notes

- [ ] Vendoring mechanism for `miniextendr`-crates
