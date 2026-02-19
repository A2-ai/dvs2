# `dvs` status

Goal: Provide an overview of the changed data files and potential files to track
via the traced data file filters.

## CLI

The option to return `--json` must be present.

```shell
$ dvs status --help
Status of the DVS repository 

Usage:
  dvs status [FILTERS] [OPTIONS]

Filters:
  --current
  --unsynced, 
  --absent
 
  -h, --help      Print help
```

When a filter is provided, only the selected state(s) are provided.



```sh
dvs status

Current files:
  <display all current files>

Changed files (unsynced):
  <display all current files> 

Absent:
  <display all absent files>
```

We do not need to display the user in unsynced files, as they are likely to be owned by the current user.

## R

Signature:

```r
dvs_status <- function(
  path = c() # paths to explit files or dirs
  glob = c(),
  recurse = FALSE # if true, any directories provided in the path should check all files within 
)
```

## Return format

### CLI JSON format

<!-- TO DISCUSS -->

### R format

Old format: `relative_path`, `status`, `file_size_bytes`, `blake3_checksum`

Proposed format:

- `absolute_path`: abbreviated when printed in R (pillar)
- `relative path`: full path
- `status`: ordered factor instead of `character()`
  - `absent|unsync|sync|present|added`
- `<alg>_checksum`: always abbreviated in print (pillar, first 12 characters)
- `size`: using units and not raw `double()/numeric()`

