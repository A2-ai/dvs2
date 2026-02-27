# get / `dvs get` / `dvs::dvs_get`

## CLI

The option to return `--json` must be present.

```shell
$ dvs get <PATHS> [OPTIONS] --help

Paths:
    Optional list of files or directories to retrieve from storage. If missing, then all files
    are retrieved, analogous to `dvs sync`

Options:
    --json Outputs are emitted in a JSON format
    --glob Specify a glob expression for which tracked datafiles and/or directories to retrieve
    --help, -h
```

`dvs get` shall accept directories as arguments, unlike `dvs add`, the activity of syncing a complete directory is within expected user workflow, while adding directories can be done unintentionally, and have ramifications on the coherence of the dvs backend/storage.

## R

Signature:

```r
#' @param paths,glob provide a list of files to sync from backend, either explicitly `files` or via glob expression `glob`.
#' @param progress provide a progress bar
#' @param parallel control number of threads to use, `FALSE`/`0`, while `TRUE` implies at most 16 threads, or provide non-zero integer for more.
dvs_get <- function(
  paths = character(),
  glob = character(),
  progress = FALSE, 
  parallel = TRUE
)
```

## Initial release concerns

### Timeout / cancellation

In case of retrieving large amount of data (either a lot of files, or big files, or both) the user may cancel the operation via `Ctrl+C`, or a connection can timeout, or similar. The `get` command ought to be able to continue from elapsed progress, and it should tell the user, that indeed,
it is resuming a previous retrieval.
