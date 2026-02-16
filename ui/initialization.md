# dvs initialization

Goal: Prepare shared storage and initialize DVS in directory

dvs initialization will create a `dvs.toml` and a directory as specified by the
shared area in the init command. The shared dir may also need to `chown` the directory
to specify certain permissions. For example, for sensitive projects, setting
ownership to a particular group, allowing write access for the group, and limiting
read access to those not in the group.

## CLI

```default
dvs init
Starts a new dvs project. This will create a `dvs.toml` file in the root folder of where the user is calling the CLI from. root folder being the place where we find a `.git` folder

Usage: dvs init [OPTIONS] <PATH>

Arguments:
  <PATH>  Where the data will be stored

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
$ dvs init
DVS Repository created
```

## R function

```r
dvs_init <- function(
    path = ".",
    storage_directory = getOption("dvs.global_storage") %||%  
        stop("must provide a storage location"),
    permissions = NULL, 
    group = NULL, 
    metadata_folder_name = NULL)
```

Example output:

```r
> dvs_init("~/Documents/projectA")
> Error: `storage_path` is missing; Please provide a location to store dvs objects.
```

```r
> dvs_init("~/Documents/projectA", storage_directory =  "~/Documents/dvs_storage")
> A DVS repository was initialised in "/Users/elea/Documents/projectA" with storage location at "/Users/elea/Documents/dvs_storage"
```

CLI users do not need the full path shown to them, but R users need that information.

## Storage

- Multiple projects can be hosted within the same storage
  - DVS storage locations should contain a list of projects it is currently serving.

### Case: No project or specific work directory

Considering the one off scripts that scientists might create, in which there is
no project surrounding where said script is.

- User/machine storage
- A remote project
- One off scripts

## Journey 1: Initial Setup with defaults

Expected outcomes:

- `dvs.toml` created in working directory
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

## Data formats to track

- `.csv`
- `.rds`
- don't track `.RDA` files, as they are a collection of datasets

Configuration: Must add these filters to the `dvs.toml`.
