# dvs spec

`dvs` (data versioning system) is a file linker that allows teams to version files under Git without directly tracking them.

`dvs` is available in 3 forms:

- Rust library in `dvs` folder
- CLI in `dvs-cli` folder
- R package in `dvs-rpkg` folder

All configuration is handled in a `dvs.toml` config file.

All CLI commands take a `--json` flag if you want to get JSON output.

The Rust library must not contain anything specific to the CLI or the R package: no `miniextendr-*` or other R FFI dependency, no `println!`/`eprintln!`/`dbg!`. It may depend on the `log` facade but must not pull in a `log` implementer; logger setup belongs in `dvs-cli` and `dvs-rpkg`.

## Glossary

- project: a folder where there is a `dvs.toml` file. `dvs` finds the project root by walking up from the current
  directory until it finds a `dvs.toml. There may be multiple projects in a single git repository for example if the data needs to be stored differently
- storage: where the data files are actually stored, currently only on a local filesystem (typically NFS shared drive)
- metadata folder: `dvs` will keep files metadata folder in a directory (by default `.dvs`) mimicking the paths to the file
- hashes: each file will be hashes and content will be stored in a place in the storage following that hash. By default it uses blake3

## High-level overview

There are 4 main actions: `init`, `add`, `get` and  `status`. We will see them more in details later

`init` will create the `dvs.toml`. 
You can pass some arguments to it or edit the toml file manually.

`add` will add and update files to the storage. In practice this will:

1. hash the files (currently blake3 only)
2. save a metadata file for each files in the metadata folder
3. add each file to a `.gitignore`

`get` will retrieve the files thanks to the metadata folder and pull them from the storage to their location in the project.

`status` is a way to check whether everything is checked out or if you have files different from what's in the metadata and
that you might want to add again.

A file in a `dvs` project can be in 4 states:

- `untracked`: not added in `dvs`
- `current`: tracked in `dvs` and we have the same version as in the metadata folder checked out
- `absent`: metadata exists in the folder but the local file is not present
- `unsynced`: local file and metadata exists but the local file differs from the metadata


## In-depth spec

### init

init will always error if a `dvs.toml` already exists in the target directory.
This check is local: a `dvs.toml` in a parent directory does not prevent initializing a nested project.

On partial failure (e.g., metadata folder or storage creation fails after `dvs.toml` is written),
init attempts best-effort cleanup of local artifacts (`dvs.toml` and, if it didn't exist beforehand, the metadata folder) so that a retry is possible.

#### CLI

```
 dvs init --help
Starts a new dvs project. This will create a `dvs.toml` file in the current folder of where the user is calling the CLI from

Usage: dvs init [OPTIONS] <PATH>

Arguments:
  <PATH>  Where the data will be stored

Options:
      --json
          Output results as JSON
      --root-dir <ROOT_DIR>
          If you want to use a root folder other than the current directory
      --metadata-folder-name <METADATA_FOLDER_NAME>
          If you want to use a folder name other than `.dvs` for storing the metadata files
      --group <GROUP>
          Unix group to set on storage directory and files
      --no-compression
          Disable compression of stored files. Compression defaults to zstd
  -h, --help
          Print help
```

#### Rust library

Library takes a project directory and the config to save.

#### R package

```r
dvs_init(storage_path, root_dir = ".", group = NULL, metadata_folder_name = NULL, no_compression = FALSE)
```

- `storage_path`: where the data will be stored (required, same as CLI's `<PATH>`)
- `root_dir`: project root where `dvs.toml` is created (defaults to working directory)
- `group`: Unix group to set on storage directory and files
- `metadata_folder_name`: custom name for the metadata folder (default `.dvs`)
- `no_compression`: disable zstd compression of stored files

Returns a list with `status = "initialized"`.

### add

It only takes files as input, directories will not work unless combined with a glob. It can also take an optional
message that will be recorded in the metadata file.

This method follows a best-effort approach: even if some files failed to be added, it will still try to add everything
and not stop.

Each file in an `add` result reports an outcome:

- `copied`: the file was new or had changed and was copied to storage
- `present`: the file's hash and size match the existing metadata -> no-op. The metadata file is not
  rewritten, so fields like `message` are not updated

Symlinks are resolved before adding. If a symlink target resolves to a path outside the project root, the file is rejected.

Each `add` operation is atomic: the storage write and metadata update either both succeed or both roll back. A
failure writing to storage will not leave behind a partial metadata file, and vice versa.

You can also do a dry run from the CLI or the library that will return the outcome that would have happened  for each file but 
without actually doing them.

#### CLI

```
Adds the given files to dvs. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided

Usage: dvs add [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  

Options:
      --glob <GLOB>        
      --json               Output results as JSON
  -m, --message <MESSAGE>  An optional message to add
      --dry-run            Show what would be added without making any actual changes
  -h, --help               Print help
```

You can run `dvs add *.csv` and it will be expanded by your shell before calling `dvs`.
To ensure globs are consistent with the R package, you can use the `--glob` parameter which will be expanded by the library.

This will exit with `1` if one or more files could not be added to the storage (file does not exist, no permissions etc).

#### Rust library

The library automatically sets up parallelism and the only error it can return is if it couldn't set up the threadpool.
It otherwise returns a list of results sorted alphabetically by path, letting users decide what to do with each.

#### R Package

```r
dvs_add(files = character(0), message, glob = NULL, dry_run = FALSE)
```

- `files`: character vector of file paths to add (can be empty if `glob` is provided)
- `message`: optional message recorded in the metadata file. Omit or pass `NULL` to skip
- `glob`: pattern to match files (same resolution rules as CLI `--glob`)
- `dry_run`: if `TRUE`, returns what would be added without making changes

Returns a data frame with one row per file. Errors if no files match.
Glob resolution uses the same rules as the CLI — see the Globbing section.

### get

`get` retrieves files from the storage into the project.
Each file in a `get` result reports an outcome:

- `copied`: the file was retrieved from storage
- `present`: the local file already matches the metadata -> noop

After retrieval, the file's hash is verified against the metadata. If it doesn't match, the retrieved file
is deleted and the operation fails for that file.

Users can use `get` with specific paths or globs. In practice those will be ran on the metadata folder rather
than the actual project, to know what to pull but the resolution works the same way as `add`.

You can also do a dry run from the CLI or the library that will return the outcome that would have happened for each file but
without actually doing them.

#### CLI

```
 dvs get --help
Retrieves the given files from dvs storage. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided

Usage: dvs get [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  

Options:
  -g, --glob <GLOB>  
      --json         Output results as JSON
      --dry-run      Show what would be retrieved without making any actual changes
  -h, --help         Print help
```

This will exit with `1` if one or more files could not be retrieved.

#### Rust library
The library automatically sets up parallelism and the only error it can return is if it couldn't set up the threadpool.
It otherwise returns a list of results sorted alphabetically by path, letting users decide what to do with each.

#### R package

```r
dvs_get(files = character(0), glob = NULL, dry_run = FALSE)
```

- `files`: character vector of file paths to retrieve (can be empty if `glob` is provided)
- `glob`: pattern to match files in the metadata folder (same resolution rules as CLI `--glob`)
- `dry_run`: if `TRUE`, returns what would be retrieved without making changes

Returns a data frame with one row per file. Errors if no files match.

### status

This returns the status (mentioned in the high level overview above) of the tracked files in the project.

#### CLI

```
 dvs status --help
Gets the status of each tracked file in the current repository.
By default shows all tracked files; use the flags below to filter.

Usage: dvs status [OPTIONS]

Options:
      --current   Include the files that are current
      --json      Output results as JSON
      --absent    Include the files that are absent
      --unsynced  Include the files that are unsynced
  -h, --help      Print help
```

By default (no flags), `dvs status` shows all tracked files regardless of state. The `--current`, `--absent`, and
`--unsynced` flags are filters: when one or more are provided, only files matching those states are shown.

#### Rust library

The library scans all metadata files in the project and returns a result for each one. Like `add` and `get`,
it never errors as a whole — individual files that cannot be inspected (e.g. permission errors) are reported
as per-file errors in the result list.

#### R package

```r
dvs_status(current = FALSE, absent = FALSE, unsynced = FALSE)
```

- `current`, `absent`, `unsynced`: filter flags. When all are `FALSE` (default), all tracked files are returned. When one or more are `TRUE`, only files matching those states are returned. Errors are always included.

Returns a data frame with one row per tracked file.

## Internals

### Metadata file format

Each tracked file has a corresponding `.dvs` sidecar file stored in the metadata folder (mirroring the
project's directory structure). For example, `data/input.csv` has metadata at `.dvs/data/input.csv.dvs`.

The sidecar is a JSON file with the following fields:

```json
{
  "hashes": {
    "blake3": "<64-char hex string>"
  },
  "size": 12345,
  "created_by": "username",
  "add_time": "2026-02-27T09:55:51.980811622Z",
  "compression": "zstd",
  "message": "optional user-provided message"
}
```

- `hashes.blake3`: always present
- `size`: file size in bytes
- `created_by`: system username at the time of `add`, `unknown` if it can't be found
- `add_time`: ISO 8601 timestamp
- `compression`: `"zstd"` or `"none"` — records how this file is stored in the backend
- `message`: omitted from JSON when not provided

Two metadata files are considered equal if their `hashes` and `size` match (other fields are informational).

### Storage layout

Storage is content-addressable: files with identical content share the same storage blob regardless of
their path or name. Files are stored using their blake3 hash split into a 2-character directory prefix
and the remaining characters as the filename.
For example, a file with blake3 hash `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` is stored at
`<storage-path>/af/1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.

Files in storage are compressed with zstd by default. The two compression modes are `zstd` and `none`
(set at `init` time via `--no-compression`, or by editing `dvs.toml`). The compression method is recorded
per-file in the metadata, so changing the project's compression setting does not affect retrieval of
previously added files — `get` always reads the compression from the file's metadata.

Writes to storage are atomic: data is first written to a temporary file (with a `.tmp` extension) in the
same directory, then renamed into place. Since rename is atomic on POSIX within the same filesystem, this
prevents partial blobs from appearing in storage mapping to an expected file.

Stored files are set read-only after writing.


### Audit trail

Every `add` operation is logged to an append-only audit file (`audit.log.jsonl`) in the storage directory. Each
entry records:

- `operation_id`: a UUID grouping all files from one `add` invocation
- `timestamp`: unix seconds
- `user`: the system username of whoever ran the command
- `file`: path and hashes of the added file
- `action`: currently only `add`

The audit log is protected by a mutex so a single `dvs` process cannot corrupt it but there is no protection against
multiple processes appending logs.

### Hash cache

`dvs` maintains a SQLite cache at `{metadata_folder}/.cache/dvs.db` to avoid re-hashing files that haven't
changed. Both `add` and `get` populate the cache after a successful operation, so a subsequent `status`
check benefits from cache hits. A cache hit is determined by matching the file's `mtime` and `size`. The cache directory is
automatically added to `.gitignore` on creation. The cache is entirely optional — if it can't be opened (e.g. corruption), `dvs` deletes it and retries once,
and operations proceed without it if that also fails.

### Parallelism

`add`, `get`, and `status` run file operations in parallel. You can set the `DVS_NUM_THREADS` environment
variable to control the thread count. If it is unset, the default is `min(available_parallelism * 4, 16)`.
If it is set to a positive integer, the override is capped at 32. In both cases the final thread count is
clamped to the number of files being processed.

### Gitignore

After a successful `add`, each added file is appended to a `.gitignore` in the file's parent directory
using the format `/<filename>` unless it's already present. If the repository has no `.git` folder, gitignore
updates are skipped entirely. A failure to update `.gitignore` is logged as a warning but does not cause
the `add` operation to fail.

### Globbing

`add` and `get` both accept a `--glob` flag. The resolution works the following way:

- Explicit files: added/retrieved directly (glob ignored)
- Explicit directories with a glob: walked and filtered by glob
- No given paths with a glob: walks current directory filtered by glob

Globs use a literal path separator, meaning `*.csv` only matches files in the target directory and
will not match `subdir/file.csv`. Use `**/*.csv` to match recursively across subdirectories.
