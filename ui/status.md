# `dvs` status

```r
dvs_status <- function(
    path = ".",
    show_storage = FALSE,
)
```

- `show_storage`:
  - Show location of storage(s) for the current dvs repository.
  - Show number of projects that the storage contains

## Data name format

`dvs_status` should show untracked data files in the current dvs repository, if
tracking is specified.

`dvs_track(".csv")`: tracks all CSV files.

`dvs_track("model_data/*")`: all files in a directory will be added to the (potentially untracked files)

`dvs_track("results/*.rds")`: glob on all r data that are saved in a specific directory.
