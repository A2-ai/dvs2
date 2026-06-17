# dvs spec

`dvs` (data versioning system) is a file linker that allows teams to version files under Git without directly tracking them.

`dvs` is available in 3 forms:

- Rust library in `dvs` folder
- CLI in `dvs-cli` folder
- R package in `dvs-rpkg` folder

All configuration is handled in a `dvs.toml` config file.

All CLI commands take a `--json` flag if you want to get JSON output.

The Rust library must not contain anything specific to the CLI or the R package: no `miniextendr-*` dependency, no `println!`/`eprintln!`/`dbg!`. It may depend on the `log` facade but must not pull in a `log` implementer. Logger setup belongs in `dvs-cli` and `dvs-rpkg`.

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
3. add each metadata file to `.gitignore` that are adjacent to the files

`get` will retrieve the files thanks to the metadata folder and pull them from the storage to their location in the project.

Both `add` and `get` support a dry run from the CLI, the library, or the R package, returning the outcome that would have happened for each file without actually doing it.

`status` is a way to check whether everything is checked out or if you have files different from what's in the metadata and
that you might want to add again.

A file in a `dvs` project can be in 3 states:

- `current`: tracked in `dvs` and we have the same version as in the metadata folder checked out
- `absent`: metadata exists in the folder but the local file is not present
- `unsynced`: local file and metadata exists but the local file differs from the metadata

### Relative paths vs. absolute paths

All `dvs` commands must accept absolute paths, and whether they need to be within a project scope, or symlinked, or otherwise should be handled by the library. Similarly, relative paths are supported.

### R package

Use `NULL` in `dvs_*` functions, when we want to defer to the default value
by the library.


## In-depth spec

### Errors / failures

All input must be evaluated, and errors as well as successes must be collected and then reported back to the user.

#### CLI

Error handling in CLI must be designed around the use case of being a part
of pipelines, or non-interactive workflows. Returning error codes is necessary.

#### R package

Error handling in the R package assumes interactive, long-lived sessions. Thus,
the R package must provide comprehensive information, that the end-user can
explore.

### init

init will always error if a `dvs.toml` already exists in the target directory.
This check is local: a `dvs.toml` in a parent directory does not prevent initializing a nested project.

init also rejects a storage path inside the repository root. This guard applies to local-filesystem backends.
Backends with no local directory (such as a future remote backend) skip it.

On partial failure (e.g., metadata folder or storage creation fails after `dvs.toml` is written),
init attempts best-effort cleanup of local artifacts (`dvs.toml` and, if it didn't exist beforehand, the metadata folder) so that a retry is possible.

#### CLI

```shell
❯ dvs init --help
Starts a new dvs project. This will create a `dvs.toml` file in the current folder of where the user is calling the CLI from

Usage: dvs init [OPTIONS] <PATH>

Arguments:
  <PATH>  Where the data will be stored

Options:
      --json
          Output results as JSON
      --threads <THREADS>
          Number of threads for parallel operations (0 = auto-detect)
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

Library takes a project directory and the configuration of the backend to initiate repository.

#### R package

```r
dvs_init(
  storage_path,
  root_dir = NULL,
  group = NULL,
  metadata_folder_name = NULL,
  compression = c("zstd", "none")
)
```

- `storage_path`: where the data will be stored (required, same as CLI's `<PATH>`)
- `root_dir`: project root where `dvs.toml` is created (defaults to working directory)
- `group`: Unix group to set on storage directory and files
- `metadata_folder_name`: custom name for the metadata folder (default `.dvs`)
- `compression`: desired compression for the stored data (default `zstd`)

Returns a list with `status = "initialized"`, invisibly.

### add

It only takes files as input, directories will not work unless combined with a glob. It can also take an optional
message that will be recorded in the metadata file. Similarly, `add` must not have a recursive option. The glob mechanism is sufficient, and intentional when used.

This method follows a best-effort approach: even if some files failed to be added, it will still try to add everything
and not stop.

Each added file gets a metadata sidecar at `<metadata folder>/<path to file>.dvs` (metadata folder defaults to
`.dvs`), mirroring the added file. The sidecar must be committed to version control (e.g. `git`). The data file itself is gitignored.

Each file in an `add` result reports an outcome:

- `copied`: the file was new or had changed and was copied to storage
- `present`: the file's hash and size match the existing metadata -> no-op. The metadata file is not
  rewritten, so fields like `message` are not updated

Symlinks are resolved before adding. If a symlink target resolves to a path outside the project root, the file is rejected.

Each `add` operation is atomic: the storage write and metadata update either both succeed or both roll back. A
failure writing to storage will not leave behind a partial metadata file, and vice versa.

#### CLI

```shell
❯ dvs add --help
Adds the given files to dvs. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided

Usage: dvs add [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  

Options:
      --json               Output results as JSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
      --glob <GLOB>        
  -m, --message <MESSAGE>  An optional message to add
      --dry-run            Show what would be added without making any actual changes
  -h, --help               Print help
```

You can run `dvs add *.csv` and it will be expanded by your shell before calling `dvs`.
To ensure globs are consistent with the R package, you can use the `--glob` parameter which will be expanded by the library. The glob string must be quoted (for example `--glob '*.csv'`) so the shell does not expand it before `dvs` sees it.

This will exit with `1` if one or more files could not be added to the storage (file does not exist, no permissions, etc).

#### Rust library

The library automatically sets up parallelism and the only error it can return is if it couldn't set up the threadpool.
It otherwise returns a list of results sorted alphabetically by path, letting users decide what to do with each.

#### R package

```r
dvs_add(paths = character(0), message = NULL, glob = NULL, dry_run = NULL)
```

- `paths`: character vector of file paths to add (can be empty if `glob` is provided)
- `message`: optional message recorded in the metadata file.
- `glob`: pattern to match files (same resolution rules as CLI `--glob`)
- `dry_run`: if `TRUE`, returns what would be added without making changes

Returns a data frame with one row per file. Errors if no files match.
Glob resolution uses the same rules as the CLI. See the Globbing section.

Partial completion leads to a return value as a list containing two elements named `result`
and `error`, each containing data-frames describing the failures and successes.

### get

`get` retrieves files from the storage into the project.
Each file in a `get` result reports an outcome:

- `copied`: the file was retrieved from storage
- `present`: the local file already matches the metadata -> noop

After retrieval, the file's hash is verified against the metadata. If it doesn't match, the retrieved file
is deleted and the operation fails for that file.

Users can use `get` with specific paths or globs. In practice those will be ran on the metadata folder rather
than the actual project, to know what to pull but the resolution works the same way as `add`.

#### CLI

```shell
❯ dvs get --help
Retrieves the given files from dvs storage. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. At least one path or --glob must be provided; to restore every tracked file, pass `--glob '**/*'`

Usage: dvs get [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  

Options:
      --json               Output results as JSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
  -g, --glob <GLOB>        
  -r, --recursive          Recursively include files in subdirectories for directory inputs. Without this flag, directories return only their direct children. Has no effect when no explicit paths are given
      --dry-run            Show what would be retrieved without making any actual changes
  -h, --help               Print help
```

Passing `-r, --recursive` with an explicit directory walks its subdirectories. It only constrains paths given
explicitly, so an empty path list still returns every tracked file.

This will exit with `1` if one or more files could not be retrieved.

#### Rust library

The library automatically sets up parallelism and the only error it can return is if it couldn't set up the threadpool.
It otherwise returns a list of results sorted alphabetically by path, letting users decide what to do with each.

#### R package

```r
dvs_get(paths = character(0), glob = NULL, recursive = NULL, dry_run = NULL)
```

- `paths`: character vector of file paths to retrieve (can be empty if `glob` is provided)
- `glob`: pattern to match files in the metadata folder (same resolution rules as CLI `--glob`)
- `recursive`: if `TRUE`, walk explicit directory paths recursively (same as the CLI `-r, --recursive`)
- `dry_run`: if `TRUE`, returns what would be retrieved without making changes

Returns a data frame with one row per file. Errors if no files match.

Partial completion leads to a return value as a list containing two elements named `result`
and `error`, each containing data-frames describing the failures and successes.

### status

This returns the status (mentioned in the high level overview above)
 of the tracked files in the project.

#### CLI

```shell
❯ dvs status --help
Gets the status of each files in the current repository

Usage: dvs status [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  Paths (files or directories) to check status for

Options:
      --json               Output results as JSON
      --threads <THREADS>  Number of threads for parallel operations (0 = auto-detect)
  -r, --recursive          Recursively include files in subdirectories for given directories
      --current            Include the files that are current
      --absent             Include the files that are absent
      --unsynced           Include the files that are unsynced
      --with-metadata      Show all metadata columns in the table output
  -h, --help               Print help
```

By default (no flags), `dvs status` shows all tracked files regardless of state. The `--current`, `--absent`, and
`--unsynced` flags are filters: when one or more are provided, only files matching those states are shown.

This will exit with `1` if one or more files could not be inspected. An explicitly listed path that matches no tracked
file is also an error and causes a non-zero exit. Such a path is reported per path while the tracked files in the same
command are still shown. A glob given without an explicit path that matches nothing is not an error for `status`. It
prints an empty result and exits `0`, because a read-only pattern scan with zero matches is a valid outcome.

#### Rust library

The library scans all metadata files in the project and returns a result for each one. Like `add` and `get`,
it never errors as a whole. Individual files that cannot be inspected (e.g. permission errors) are reported
as per-file errors in the result list.

#### R package

```r
dvs_status(
  paths = character(0),
  recursive = NULL,
  status = c("current", "absent", "unsynced")
)
```

- `paths` optional vector of paths files to retrieve status of
- `recursive` if `TRUE`, recursively include files in subdirectories for any directory paths
- `status` represent the filter flags `current`, `absent`, `unsynced`
  When omitted, all three states are present in the returned data-frame, otherwise serval filtering status can be provided

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
- `compression`: records how this file is stored in the backend (`"zstd"` or `"none"`)
- `message`: omitted from JSON when not provided

Two metadata files are considered equal if their `hashes` and `size` match (other fields are informational).

### Storage layout

Storage is content-addressable: files with identical content share the same storage blob regardless of
their path or name. Files are stored using their blake3 hash split into a 2-character directory prefix
and the remaining characters as the filename.
For example, a file with blake3 hash `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` is stored at
`<storage-path>/af/1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.

Writes to storage are atomic: data is first written to a temporary file (with a `.tmp` extension) in the
same directory, then renamed into place. Since rename is atomic on POSIX within the same filesystem, this
prevents partial blobs from appearing in storage mapping to an expected file.

Stored files are set read-only after writing.

### Compression

The two compression modes are `zstd` (default) and `none`, set at `init` time (CLI
`--no-compression`, R `compression`) or in `dvs.toml`. The mode is recorded per-file in the
metadata, so `get` always reads it from there. Changing the project setting (`dvs.toml`) must not
affect previously added files.

### Audit trail

Every `add` and `init` operation is logged to an append-only audit file (`audit.log.jsonl`) in the storage directory.
Each entry records:

- `operation_id`: a UUID grouping all files from one operation
- `timestamp`: unix seconds
- `user`: the system username of whoever ran the command
- `action`: either `add` or `init`

The `add` action is an object that contains

- `file`: path and hashes of the added file
- `compression`: `"zstd"` or `"none"`, see the Compression section

The `init` action is an object that contains

- `settings`: the project configuration written to `dvs.toml`
- `project_path`: the absolute path of the initialized project

The audit log is protected by a mutex so a single `dvs` process cannot corrupt it but there is no protection against
multiple processes appending logs.

Example:

```json
{
  "operation_id": "93ca9155-b767-47e8-b433-a110f5943212",
  "timestamp": 1776959650,
  "user": "elea",
  "action": {
    "add": {
      "file": {
        "path": "data/derived/many34.csv",
        "hashes": {
          "blake3": "55539acdd1e0617c9180f8a593aeda480b7fab415faef16596f25098908b8487"
        }
      },
      "compression": "zstd"
    }
  }
}
```

### Hash cache

`dvs` maintains a SQLite cache at `{metadata_folder}/.cache/dvs.db` to avoid re-hashing files that haven't
changed. Both `add` and `get` populate the cache after a successful operation, so a subsequent `status`
check benefits from cache hits. A cache hit is determined by matching the file's `mtime` and `size`. The cache directory is
automatically added to `.gitignore` on creation. The cache is optional.

### Parallelism

`add`, `get`, and `status` run file operations in parallel. You can set the `DVS_NUM_THREADS` environment
variable to control the thread count, if the CLI and R package have not set the number
of threads directly. If it is unset, the default is `min(available_parallelism * 4, 16)`.
If it is set to a positive integer, the override is capped at 32. In both cases the final thread count is
clamped to the number of files being processed.

#### R package

The threadpool size set directly by the R package takes precedence over the
`DVS_NUM_THREADS` environment variable.

### Gitignore

After a successful `add`, each data file gets a `/<filename>` entry in its own directory's `.gitignore`, keeping the
data out of Git while the `.dvs` metadata stays tracked. Existing entries are not duplicated. If there is no `.git`
folder the update is skipped, and a failed `.gitignore` update is logged as a warning without failing the `add`.

### Globbing

`add`, `status`, and `get` accept a `--glob` flag. The resolution works the following way:

- Explicit files: added/retrieved directly (glob ignored)
- Explicit directories with a glob: walked and filtered by glob
- No given paths with a glob: walks current directory filtered by glob

Globs use a literal path separator, meaning `*.csv` only matches files in the target directory and
will not match `subdir/file.csv`. Use `'**/*.csv'` (quoted, to avoid shell expansion) to match recursively across subdirectories.

`status` and `get` also accept `-r/--recursive`, which walks the given directories instead of matching a glob pattern. `--recursive` and `--glob` are mutually exclusive. `add` accepts `--glob` but has no `--recursive` flag.
