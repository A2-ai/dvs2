# get / `dvs get` / `dvs::dvs_get`

## CLI

The option to return `--json` must be present.

```shell
$ dvs get <PATHS> [OPTIONS] --help

Paths:
    Optional list of files to retrieve from storage. If missing, then all files
    are retrieved, analogous to `dvs sync`

Options:
    --json Outputs are emitted in a JSON format
    --glob Specify a glob expression for which tracked datafiles to retrieve
    --help, -h
```

## R

Signature:

```r
dvs_get <- function(
  files = character(),
  glob = character(),
)
```
