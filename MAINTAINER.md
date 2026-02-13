# DVS Maintainer Guide

## Version Management

The `bump-version` tool syncs versions across DESCRIPTION, Cargo.toml, Cargo.toml.in, and configure.ac files.

### Usage Examples

```bash
just bump-version                      # sync versions from DESCRIPTION
just bump-version -- --bump=patch      # 0.0.0.9000 -> 0.0.1
just bump-version -- --bump=minor      # 0.0.0.9000 -> 0.1.0
just bump-version -- --bump=dev+       # 0.0.0.9000 -> 0.0.0.9001
just bump-version -- --set=0.1.0       # set specific version
just bump-version -- --mode=workspace  # only update [workspace.package]
```

### Version Format

- **R package (DESCRIPTION)**: `0.1.0.9000` (4-part dev versions allowed)
- **Cargo.toml**: `0.1.0-9000` (4th segment uses dash, not dot)
- **configure.ac**: `0.1.0.9000` (matches DESCRIPTION)

### Files Updated

| File | Format |
|------|--------|
| `dvs-rpkg/DESCRIPTION` | R version (source of truth) |
| `dvs-rpkg/configure.ac` | R version |
| `dvs-rpkg/src/rust/Cargo.toml.in` | Cargo version |
| `dvs-rpkg/src/rust/Cargo.toml` | Cargo version (generated) |
| `Cargo.toml` | Cargo version (workspace) |
