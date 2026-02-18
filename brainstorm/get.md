# `dvs get`

## CLI

The option to return `--json` must be present.

## R

Signature:

```r
dvs_get <- function(
  files = character(),
  glob = character(),
  ignore.case = NULL %||% !is.empty(glob),
  fail = FALSE # follows fs::dir_ls
)
```


