# dvs initialization / `dvs init` / `dvs_init`

Goal: Prepare shared storage and initialize DVS in directory

dvs initialization will create a `dvs.toml` and a directory as specified by the
shared area in the init command. The shared dir may also need to `chown` the directory
to specify certain permissions. For example, for sensitive projects, setting
ownership to a particular group, allowing write access for the group, and limiting
read access to those not in the group.

## User site assumptions

- Always operating within a repository/project/workspace, and will initialize 
  at current working directory.
- Storage is detached from repository root

## CLI

```shell
dvs --- Data version control and storage management system

Usage:
  dvs <COMMANDS> [OPTIONS]

Commands:
  init
  add
  get
  status
  audit
  log

Options:
  -h, --help    Show help for command (e.g. `dvs init --help`)
  --version     Show version information
```

The initialization command will have further subcommands.

```shell
dvs init --- Initialize a new DVS repository

Usage:
  dvs init <BACKEND> [OPTIONS]

Backends:
  fs     Local, on-disk storage backend

Options:
  -h, --help    Show help for command (e.g. `dvs init --help`)
```

### fs

```shell
dvs init fs --- Initialize a DVS repository via on-disk storage

Usage:
  dvs init fs <storage-path> [OPTIONS]

Required:
  <storage-path>    path to the local storage locations (e.g. `/data/`)

Options:
  --json
      Output results as JSON
  --metadata-folder-name <METADATA_FOLDER_NAME>
      If you want to use a folder name other than `.dvs` for storing the metadata files
  --permissions <PERMISSIONS>
      Unix permissions for storage directory and files (octal, e.g., "770")
  --group <GROUP>
      Unix group to set on storage directory and files
  --no-compression
      Disable compression of stored files. Compression defaults to zstd
  -h, --help
          Print help
```

Example output:

```shell
$ dvs init /data/
DVS Repository created with storage path located at <ABSOLUTE STORAGE PATH>
```

## R function

```r
dvs_init <- function(
  storage_path = character(), # required
  permissions = NULL, 
  group = NULL, 
  metadata_folder_name = NULL)
```

Example output:

```r
> dvs_init()
> Error: `storage_path` is missing; Please provide a location to store dvs objects.
```

```r
> dvs_init("/data/projectA_storage")
> A DVS repository was initialized in "/Users/elea/Documents/projectA" with storage location at "/data/projectA_storage"
```

CLI users do not need the full path shown to them, but R users need that information.

Different storage backends have to be initialized through specialized functions.

- `dvs_init_fs` with alias `dvs_init`

## Journey 1: Initial Setup with defaults

Expected outcomes:

- `dvs.toml` created in the ancestral directory that contains `.git`, or other heuristics.
- shared dir created in specified path, with default permissions of 664

Known Caveats:

- certain linux `umask` setups cause folders to have default permissions like 600, or 644
where other collaborators could not write by default, therefore,

### CLI flow

1. initialize dvs from a project directory

```bash
dvs init /data/dvs/example-proj
```

### R package flow

1. Initialize DVS in the repo

```r
dvs_init("/data/shared/project-x-dvs")
```

## Journey 2: Initial Setup with shared folder locked down to group

- set permissions to writeable by group, not readable if not in group (660)
- group name projx

Expected outcomes:

- dvs.toml created in working directory
- shared dir created in specified path, with permissions of 660 and owned by group projx

Edge cases:

- group must resolve to known gid on system

### CLI flow

1. initialize dvs from a project directory

```bash
dvs init /data/dvs/sensitive-projx --permissions "660" --group projx
```

### R package flow

1. Initialize DVS in the repo

```r
dvs_init("/data/shared/project-x-dvs", permissions = "660", group = "projx")
```

#### Returns

Return a rich data-frame that the end-user can then further subset/filter
to fit their needs.

Old format: `relative_path`, `outcome`, `file_size_bytes`, `blake3_checksum`.

- [ ] New format:
  - `absolute_path`: abbreviated when printed in R (pillar)
  - `relative path`: full path
  - `status`: ordered factor instead of `character()`
  - `absent|unsync|sync|present|added`
  - `checksum`: always abbreviated in print (pillar, first 5 characters)
  - `size`: using units and not raw `double()/numeric()`

## Data formats to track

- `.csv`
- `.rds`
- don't track `.RDA` files, as they are a collection of datasets

Configuration: Must add these filters to the `dvs.toml`.

Known annoyance: Verbosity of this can be annoying.
There should be a way to reduce outputs on untracked data files available
to the user.
