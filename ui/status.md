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
  --unsynced, --missing
  --absent
  --no-current
  --no-unsynced
  --no-absent

Options:
  -s, --state     filter for states to retain
  -i, --invert    inverts the selection provided by `--state`
  -h, --help      Print help
```

When a filter is provided, only the selected state(s) are provided.



```sh
dvs status

Current files:
  <display all current files tracked>

Changed files (unsynced):
  new_scenario/model_spec.txt

Untracked and followed files:
  <size in MB> orignal_scenario/model_summary.txt
  <size in MB> orignal_scenario/tab-0123.tsv
  <size in MB> orignal_scenario/tab-0123b.tsv
  <size in MB> orignal_scenario/tab-0123c.tsv
```

We do not need to display the user in unsynced files, as they are likely to be owned by the current user.

## R

Signature:

```r
dvs_status <- function(
  path = ".",
  show_storage = FALSE,
)
```

- `show_storage`:
  - Show location of storage(s) for the current dvs repository.
  - Warn the user that they must not alter the state of
  the storage directory.
  - (future) Show number of projects that the storage contains

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
- `checksum`: always abbreviated in print (pillar, first 5 characters)
- `size`: using units and not raw `double()/numeric()`

## Data name format

`dvs_status` should show untracked data files in the current dvs repository, if
tracking is specified.

## Granularity

We expect the end user to use `{dplyr}` in order to
filter to users, groups, and/or folders. Therefore it is important to provide consistent data-frames.

## Following Filters in Status

`dvs_track(".csv")`: tracks all CSV files.

`dvs_track("model_data/*")`: all files in a directory will be added to the (potentially untracked files)

`dvs_track("results/*.rds")`: glob on all r data that are saved in a specific directory.

These should result in additions to `[following]` table in `dvs.toml`. See [Following Formats](tracking.md).
