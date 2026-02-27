# `dvs add`

Goal: Add files to an initialized dvs project.

## CLI

```sh
dvs add <FILES> [OPTIONS]

Files:
    List of files to add

Options:
    -m, --message  Message attributed to the add event
    --glob         Glob expression of the files to add, e.g. (`/phaseA/**/*.csv`)
    --json Command output as JSON format
    -h, --help

```

`dvs add` will not accept directories as input. However, if all files following a glob expression is to be added, the end-user can use `--glob`/`glob=` in the cli and R package resp.

## R

Signature:

```r
dvs_add <- function(
  files = character(), # either files or glob are required
  message, # not required, can be missing, `NULL`, "", and character() (empty)
  glob = character(), # either files or glob are required
  parallel = TRUE # enable parallel
)
```
