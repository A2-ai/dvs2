# `dvs revert`

## CLI

The option to return `--json` must be present.

## R

Signature:

```r
dvs_revert <- function(path = ".",
  commit_sha = integer(),
  date = NULL,
  before = NULL, # date | duration
)
```

- `path` is location of dvs repository; the `dvs.toml` has to be present
  in an ancestor to `path`.
