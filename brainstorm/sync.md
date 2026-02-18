# `dvs sync`

Goal: Provide a streamlined way to update a cloned dvs repository.

Synchronization `sync` is an alias for `dvs get **/*`, meant as a
repository wide syncing from storage (local/remote).

## CLI

The option to return `--json` must be present.

```sh
$ dvs sync
[status] [Last modified] [Message]
...      ...            ...
```

The sync subcommand should also be able to act as a repository revert,
and 

```sh
$ dvs sync --before <duration/date>
[status] [Last modified] [Message]
...      ...            ...
```

## R

Signature:

```r
dvs_sync <- function(
  by_folder = character(),
  since = NULL, # date | duration (unit)
  recurse = TRUE ,
)
```

- `path` is a location within a dvs repository.
  Not necessarily the root a dvs repository.
- `by_folder` allows to sync specific folders only
- `recurse` is whether to sync folders recursively

### `recurse`

When there is no `by_folder`, recurse will update the entire dvs repository, even if
current directory is a sub-directory in a dvs repository. The current location of the
user might be incidental to their intent with dvs.
