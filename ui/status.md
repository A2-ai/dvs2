# `dvs` status

## CLI

The option to return `--json` must be present.

## R

Signature:

```r
dvs_status <- function(
  path = ".",
  show_storage = FALSE,
  by_folder = character(),
  by_user = character(),
)
```

- `path` is location of dvs repository; the `dvs.toml` has to be present
  in an ancestor to `path`.
- `show_storage`:
  - Show location of storage(s) for the current dvs repository.
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

`dvs_track(".csv")`: tracks all CSV files.

`dvs_track("model_data/*")`: all files in a directory will be added to the (potentially untracked files)

`dvs_track("results/*.rds")`: glob on all r data that are saved in a specific directory.
