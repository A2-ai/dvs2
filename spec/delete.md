# `dvs delete`

Goal:  tracked files.

## CLI

```shell
$ dvs delete

```

## R package

```r
dvs_delete <- function(
  files = character(), 
  glob = character()
)
```

Aliases: `dvs_delete`, `dvs_remove`, `dvs_rm`.

- `files`: list of files that are to be deleted.

### Non-existing files

Emit a warning, but still remove the files that do exist and are tracked.
