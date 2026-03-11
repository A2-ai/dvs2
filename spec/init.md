# `dvs init`

Creates a `dvs.toml` configuration file and prepares the storage backend.

## Behavior

- Errors if `dvs.toml` already exists in the target directory.
- A `dvs.toml` in a parent directory does not prevent initializing a nested project.
- Creates the storage directory if it does not exist.
- Creates the metadata folder (default `.dvs`) in the project root.
- On partial failure, attempts best-effort cleanup of `dvs.toml` and the metadata folder so a retry is possible.
- Git is not required. DVS operates independently of git.
- The storage directory should be dedicated to one project. Sharing storage across projects complicates state management.

### Storage layout

Storage uses content-addressable paths derived from blake3 hashes. Files are split into a 2-character directory prefix and the remaining hash as filename. See `specs.md` for full details.

### Compression

Default: zstd. Can be disabled with `--no-compression`. The setting is recorded per-file in metadata, so changing it after init does not affect previously stored files.

### Group ownership

When `--group` is specified, the storage directory and files are set to the given Unix group. The group must resolve to a known GID on the system.

## CLI

```
dvs init [OPTIONS] <PATH>

Arguments:
  <PATH>                    Path to the storage directory

Options:
      --root-dir <ROOT_DIR>
          Project root directory (default: current directory)
      --metadata-folder-name <NAME>
          Metadata folder name (default: .dvs)
      --group <GROUP>
          Unix group for storage directory and files
      --no-compression
          Disable compression (default: zstd)
      --json
          Output as JSON
  -h, --help
          Print help
```

### Output

Default:
```
DVS Initialized at "<absolute path>"
```

JSON (`--json`):
```json
{"status": "initialized"}
```

### Exit codes

- `0`: success
- `1`: error (e.g., `dvs.toml` already exists, storage path not writable)

## Rust library

```rust
pub fn init(root: &Path, config: Config) -> Result<PathBuf>
```

Takes the project root directory and a `Config`. Returns the path to the initialized project root. Config is constructed via `Config::new_local(storage_path, group)`.

## R package

```r
dvs_init(
  directory = ".",
  group = NULL,
  metadata_folder_name = NULL
)
```

- `directory`: path to the storage directory.
- `group`: optional Unix group name.
- `metadata_folder_name`: optional override for the metadata folder (default `.dvs`).

Returns a list with `status = "initialized"`.

## Configuration file

`dvs init` produces a `dvs.toml`:

```toml
compression = "zstd"

[backend]
path = "/absolute/path/to/storage"
```

When `--group` is specified:

```toml
compression = "zstd"

[backend]
path = "/absolute/path/to/storage"
group = "groupname"
```

## Examples

### Default setup

```bash
dvs init /data/dvs/project-x
```

Creates `dvs.toml` in the current directory pointing to `/data/dvs/project-x`. Storage uses zstd compression. No group restriction.

### Group-restricted setup

```bash
dvs init /data/dvs/project-x --group projx
```

Same as above, but storage directory and files are owned by group `projx`.
