# initialization of DVS repository / `dvs init` / `dvs::dvs_init`

Goal: Prepare shared storage and initialize DVS in directory

dvs initialization will create a `dvs.toml` and a directory as specified by the
storage area in the init command. The shared directory may also need to `chown` the directory
to specify certain permissions. For example, for sensitive projects, setting
ownership to a particular group, allowing write access for the group, and limiting
read access to those not in the group.

The storage directory will should not be regarded as a shared backend amongst
multiple projects. Achieving and snapshotting a project state will become
complicated, if storage directory was shared amongst other projects, concerning
different datasets (beyond those that are common amongst projects).

## User site assumptions

- Always operating within a repository/project/workspace.
- Git repository is not a requirement for a DVS repository.
- 
- We assume that storage directory is detached from project-tree. The data will
  is located in-tree, and thus the storage, which is a backend storage,
  should not appear there, under most typical projects.


## CLI

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
  <storage-path>    path to the local storage locations (e.g. `/data/dvs/projx`)

Options:
  --json
      Output results as JSON
  --root <PATH> specify the location that the DVS repository ought to be set
  --metadata-folder-name <METADATA_FOLDER_NAME>
      If you want to use a folder name other than `.dvs` for storing the metadata files
  --permissions <PERMISSIONS>
      Unix permissions for storage directory and files (octal, e.g., "770")
  --group <GROUP>
      Unix group to set on storage directory and files
  --no-compression
      Disable compression of stored files. Compression defaults to zstd
  --compression
      type of compression to use. zstd, gz
  -h, --help
          Print help
```

Example output:

```shell
$ dvs init /data/dvs/projx
DVS Repository created with storage path located at <ABSOLUTE STORAGE PATH>
```

this will instantiate a DVS repository in the current directory, while setting om a local file-system backend, for which more than one projects may be stored. The example shows a system-wide directory called `/data` that is meant to be the root to dvs file system backends.

## R function

```r
dvs_init <- function(
  storage_path,
  backend_config = fs_storage(), #default to file system storage
  metadata_folder_name = NULL,
  ...,
  dir = getwd() # default to creating in wd
  )
```

```
fs_storage <- function(
  permissions = NULL, 
  group = NULL 
) {...} 
```

```r
> dvs_init("/data/dvs/projx")
> A DVS project was initialized in "/Users/elea/Documents/projectA" with storage location at "/data/dvs/projx"
```

```r
dvs_init <- function(
  storage_path,
  storage_config = s3_storage(...), # different config functions can provide typed
  )
```

would result in the following toml config:

```
compression = "zstd"

[backend]
path = "/path/to/shared/storage
```




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
