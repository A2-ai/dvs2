# dvs spec

`dvs` (data versioning system) is a file linker that allows teams to version files under Git without directly tracking them.

`dvs` is available in 3 forms:

- Rust library in `dvs` folder
- CLI in `dvs-cli` folder
- R package in `dvs-rpkg` folder

All configuration is handled in a `dvs.toml` config file.

All CLI commands take a `--json` flag if you want to get JSON output.

The CLI JSON protocol ([docs/cli-json.md](docs/cli-json.md)) specifies machine-readable envelopes,
errors, exit statuses and discovery. Client read queries specify rich status facts, content availability,
and history.

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

`status` checks whether files are checked out as recorded or differ from the version in metadata.

A file in a `dvs` project has these status states:

- `current`: tracked in `dvs` and we have the same version as in the metadata folder checked out
- `absent`: metadata exists in the folder but the local file is not present
- `unsynced`: local content differs from current metadata

Both human-readable and machine-readable CLI status report differing content as `unsynced`.

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

##### Machine output (`--json`)

Every `--json` result is one envelope (`v`, `command`, `outcome`, `dry_run`, `counts`, `error`, `files`, ...),
error envelopes included, and carries its timing: `started_at` and `finished_at` (RFC 3339 UTC
with milliseconds, e.g. `2026-10-21T14:30:00.123Z`) and `duration_ms`. `add` and `get` results have
`bytes: {total, to_transfer, already_stored, transferred}` (dry runs too; `transferred` is what actually moved,
so `0` in a dry run; `to_transfer + already_stored = total`). Every time in JSON output is RFC 3339 UTC.

Errors have one shape. The envelope's `error` is `null` or `{code, message, hint}`; every row of every command
has `code`, `message` and `hint` (all `null` on success; `hint` is `null` where there is none). Rows have fixed
keys: a field that does not apply is `null`, never missing (an `add` error row has `outcome`, `hash`, `size`,
`stored_size` as `null`). `counts` always has `total`, `succeeded` and `failed` (get adds its detailed counts).

Backend and credentials failures share one set of codes, at the top level (`error.code`) and per row
(`code`): `auth_required` (no token, not logged in), `auth_expired` (the server said 401), `access_denied`
(403), `server_unavailable` (transport error, timeout, 5xx), `server_too_old` (an endpoint that should exist
answered 404/405), `project_not_found` (project 404), `bad_metadata` (an unreadable `.dvs` record, e.g. git
conflict markers) and `credentials_unreadable`. Command-specific codes (`no_files_selected`,
`invalid_configuration`, `invalid_arguments`, `io_error`, get's codes, ...) stay. `info --json` reports
`connection_error` as `{code, message}` (`server_unavailable`, `project_not_found` or `server_too_old`) when
`connection` is `unavailable`, else `null`; `access` is `not_logged_in` without a token, and `none` on a 403
(no membership: `connection` is `available`, exit `0`, like human mode's "no access"); a rejected login (401,
`auth_expired`) is an error envelope.

Exit codes, for every command: `0` complete; `1` failure, or some rows failed; `2` usage or configuration
(`invalid_arguments`, `invalid_configuration`); `3` `get` only, local conflicts held files back and nothing
else failed.

Machine mode never changes stored credentials: a 401 is `auth_expired` and the token stays; an unparsable
credentials store is `credentials_unreadable` and stays. Human mode still forgets a rejected token and deletes
an unparsable store, and says to run `dvs login`. Only `dvs login` and `dvs logout` change credentials on
purpose.

A server URL may carry a path prefix (a proxy, `https://host/dvs`, with or without the trailing slash): every
request, login included, goes under it.

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

Returns a single-row data frame describing the resulting configuration.

### add

It only takes files as input, directories will not work unless combined with a glob. It can also take an optional
message that will be recorded in the metadata file. Similarly, `add` must not have a recursive option. The glob mechanism is sufficient, and intentional when used.

Input paths are validated before any data is written. If any path cannot be resolved to a file to add (it does not exist, it is a directory passed without a glob, or it resolves outside the project root) the whole batch is refused and nothing is added.

Once every input resolves, the add is best-effort. A file that fails during the read or the storage write (for example a permission error) does not stop the others. That file is reported and the rest are still added.

Each added file gets a metadata sidecar at `<metadata folder>/<path to file>.dvs` (metadata folder defaults to
`.dvs`), mirroring the added file. The sidecar must be committed to version control (e.g. `git`). The data file itself is gitignored.

Each file in an `add` result reports an outcome:

- `copied`: the file was new or had changed and was copied to storage
- `present`: the file's hash and size match the existing metadata -> no-op. The metadata file is not
  rewritten, so fields like `message` are not updated

Symlinks are resolved before adding. If an explicitly named path resolves to a location outside the project root, it is rejected and the batch refused. When a directory is expanded by a glob, a matched file whose symlink target resolves outside the project root is skipped with a warning rather than aborting the batch.

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

A batch refused for an invalid input path exits with `1` and adds nothing. A batch that runs but has one or more files fail during copy (for example no permissions) adds the rest and also exits with `1`.

#### Rust library

The library validates every input path first and returns an error, refusing the whole batch, if any path is invalid. It also returns an error if it cannot set up the threadpool.
Otherwise it returns a list of results sorted alphabetically by path, letting users decide what to do with each per-file runtime failure.

#### R package

```r
dvs_add(paths = character(0), message = NULL, glob = NULL, dry_run = NULL)
```

- `paths`: character vector of file paths to add (can be empty if `glob` is provided)
- `message`: optional message recorded in the metadata file.
- `glob`: pattern to match files (same resolution rules as CLI `--glob`)
- `dry_run`: if `TRUE`, returns what would be added without making changes

Returns a data frame with one row per file. Glob resolution uses the same rules as the CLI. See the Globbing section.

It raises an error if no files match, or if any input path is invalid (the whole batch is refused and nothing is written). A file that fails after a valid batch starts (for example a permission error during the copy) does not raise. That file is one row with its `error` column set and the success columns `NA`, next to the rows that succeeded.

### get

`get` retrieves files from the storage into the project.

#### Before, and what dvs does

Each requested file is classified before anything is written:

| `before` | Meaning | What `get` does |
|---|---|---|
| `absent` | no working file | restores it |
| `current` | the working file already matches the metadata | nothing, and needs no server |
| `clean` | differs, but is exactly what dvs last wrote or added at this path in this clone (the baseline) | replaces it, e.g. after switching git branches |
| `modified` | differs, and differs from the baseline | conflict |
| `unknown` | differs, and there is no baseline | conflict |
| `not_tracked` | no metadata for the path (or a directory with no selected tracked files) | known problem |
| `not_a_file` | a directory or special file is in the way | known problem, never replaced |
| `unsafe_path` | a parent is a symlink, or the path leaves the project | known problem, never written |

Conflicts follow `on_conflict`:

- `abort` (default): if any conflict or other known problem exists (including a `.dvs` file that cannot be read,
  e.g. after a git merge conflict: `bad_metadata`), nothing is written. Every row is returned,
  with the rest held back as `batch_aborted`.
- `skip`: conflicts and known problems are left as they are; everything else is retrieved.
- `overwrite`: conflicts are replaced. Other known problems still abort.

Whatever the policy: the downloaded bytes are verified before install; the destination is checked again just
before the rename and kept if it changed meanwhile (`destination_changed`); and remote or I/O failures are per
file, so the other files still complete. A failed get leaves the existing local file unchanged.

Conflict rows report `stored_copy` (`present`, `absent`, `unknown`): whether the local bytes exist in this
store. It is a hint for the next step, not permission to replace.

The baseline is a table in the cache database (see Hash cache). It is written only after a verified install, a
`get` that found the file current, or a successful `add`. Losing it makes changed files `unknown`, which costs a
`--force`, never data.

#### Configuration

Precedence: argument/flag > `DVS_GET_ON_CONFLICT` > `dvs.toml` > `abort`.

```toml
[get]
on_conflict = "skip"   # or "abort"
```

`on_conflict = "overwrite"` in `dvs.toml` is a configuration error: a file committed to the repository must not
be able to discard someone's local changes. Use the flag or the environment variable.

#### CLI

```shell
❯ dvs get --help
Retrieves the given files from dvs storage. You can use a glob or paths. If you pass a directory and a glob, the glob will be ran from that directory. With no paths, gets the tracked files directly in the current directory; add -r for everything beneath it

Usage: dvs get [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...  

Options:
      --json                  Output results as JSON
      --threads <THREADS>     Number of threads for parallel operations (0 = auto-detect)
  -g, --glob <GLOB>           
  -r, --recursive             Recursively include files in subdirectories for directory inputs. Without this flag, directories return only their direct children
      --dry-run               Show what would be retrieved without making any actual changes
      --on-conflict <POLICY>  What to do with files whose local version dvs did not put there (local changes, or no record): abort changes nothing, skip leaves them and gets the rest, overwrite replaces them. Defaults to DVS_GET_ON_CONFLICT, then [get] on_conflict in dvs.toml, then abort [possible values: abort, skip, overwrite]
  -f, --force                 Replace files with local changes (same as --on-conflict overwrite)
  -h, --help                  Print help
```

Passing `-r, --recursive` with an explicit directory walks its subdirectories. When no explicit paths are given,
`get` is scoped to the current directory (relative to the repo root): without `-r` it restores the tracked files
directly under it, and with `-r` it restores every tracked file beneath it. (At the repo root, `-r` therefore
restores every tracked file in the project.)

Human output lists each file that changed or needs attention, then a summary naming the policy and where it came
from, then the commands to keep or discard local changes.

`--json` prints one envelope (this replaced the earlier bare array), also when `get` cannot start (not in a project,
invalid configuration): then `files` is empty, `outcome` is `failed`, `error` says why, and `on_conflict`/`source`
are null.

```json
{"v": 1, "command": "get", "outcome": "aborted", "dry_run": false,
 "on_conflict": "abort", "source": "default",
 "counts": {"total": 3, "succeeded": 1, "restored": 0, "replaced": 0, "unchanged": 1, "skipped": 0, "not_run": 2, "failed": 0, "would_restore": 0, "would_replace": 0},
 "error": null,
 "files": [{"path": "data/b.csv", "local_path": "data/b.csv", "before": "modified", "action": "not_run",
            "satisfied": false, "size": 7, "target_hash": "…", "local_hash": "…", "stored_copy": "absent",
            "code": "local_changes", "message": "…", "hint": "keep your version: dvs add data/b.csv; …"}]}
```

- `outcome`: `complete`, `aborted` (nothing written), `partial` or `failed`.
- `satisfied`: the file is at its tracked version afterwards; in a dry run, that it would be.
- A shared cause (`access_denied`, `server_unavailable`) is printed once in human output, not per file.
- `action`: `restored`, `replaced`, `unchanged`, `skipped`, `not_run`, `failed`, `would_restore`, `would_replace`.
- `code`: `local_changes`, `unknown_local`, `batch_aborted`, `not_tracked`, `not_a_file`, `unsafe_path`,
  `source_missing`, `hash_mismatch`, `access_denied`, `server_unavailable`, `destination_changed`, `permission_denied`, `file_in_use`,
  `io_error`, `bad_metadata`, and the other shared codes (`auth_required`, `auth_expired`, `server_too_old`,
  `project_not_found`, `credentials_unreadable`).
- `counts`: `total` (every row) and `succeeded` (rows at their target: restored, replaced, unchanged, and
  would_restore/would_replace in a dry run) next to the per-action counts; `failed` is the `failed` action.
  Skipped and `not_run` rows (held back) are in neither `succeeded` nor `failed`.
- `bytes`: `to_transfer` is the size of every row whose local copy does not match its target (held back,
  skipped and failed rows included), `already_stored` of the rows that match.

Exit codes: `0` complete; `1` any failure or operation problem (takes priority); `3` only conflicts held files
back; `2` usage or configuration errors.

#### Rust library

`select_paths_for_get` resolves the request and returns the tracked paths plus the unmatched ones, which become
`not_tracked` rows rather than an error. `get_files(selection, paths, backend, GetOptions { dry_run, on_conflict,
source }, on_file_start)` returns a `GetReport` with the outcome, counts, an optional operation-level `problem`,
and one `GetRow` per file sorted by path. `Config::get_on_conflict` applies the precedence and returns a
`ConfigError` for a project-sourced `overwrite`.

#### R package

```r
dvs_get(paths = character(0), glob = NULL, recursive = FALSE, dry_run = FALSE,
        on_conflict = NULL, on_error = getOption("dvs.on_error", "return"))
```

- `on_conflict`: `NULL` inherits (env, then dvs.toml, then `"abort"`), or `"abort"`, `"skip"`, `"overwrite"`.
- `on_error`: `"return"` (default) only returns the result; `"warn"` also warns; `"error"` raises. The condition
  has class `dvs_get_incomplete` and carries the result as `cnd$result`. No warning by default, so
  `options(warn = 2)` cannot prevent `res <- dvs_get()` from assigning.

It always returns a tibble of class `dvs_get_result` with the same columns as the JSON rows, also with zero rows
(for example outside a project). `dvs_result_info(res)` returns the operation information (outcome, policy and
its source, counts, problem), and printing shows it as a header. Invalid arguments and configuration errors
(`dvs_config_error`) are the only immediate errors.

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
  -g, --glob <GLOB>        
      --current            Include the files that are current
      --absent             Include the files that are absent
      --unsynced           Include files whose working content differs from metadata
      --with-metadata      Show all metadata columns in the table output
  -h, --help               Print help
```

By default (no flags), `dvs status` shows all tracked files regardless of state. The `--current`, `--absent`, and
`--current`, `--absent`, and `--unsynced` flags are filters: when one or more status flags are provided, only files matching those states are shown.

This will exit with `1` if one or more files could not be inspected.

Each named path filters independently. A path that matches no tracked file is dropped silently rather than
failing the command, so a partly misspelled invocation such as `dvs status good.bin bogus.bin` still exits
`0` and shows only the paths that matched.

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

The same database holds the `get` baseline (`baseline(path, blake3, size)`): what dvs last wrote or added at
each path. Unlike the hash cache, `status` never writes it. Deleting it is safe; changed files then count as
`unknown` until the next `get` or `add`.

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
