# `dvs delete`

Goal: Provide a controlled way to delete / untrack datafiles from DVS.

- delete from dvs storage (backend)
- delete the metadata file
- delete the data file
- delete (and commit) the `*.dvs` metafile from `git` if relevant
- document the reason behind deletion (audit log entry)

## CLI

```shell
$ dvs delete [OPTIONS] <PATHS>

Paths:
    Files to delete from the dvs backend, the current directory, delete metadata file, and also untrack metadata file from `git`

Options:
    -c, --cached do not delete the files within the project, but delete from backend and the associated metadatafile, plus add audit log entry about the deletion event.
    -m, --message (optional) message explaining why the file was deleted
    -h, --help 

```

Unlike `dvs get`, providing no arguments to `dvs delete` will not delete all the tracked files from the repository.

## R package

```r
dvs_delete <- function(
  files = character(),
  glob = character(),
  delete_cached = TRUE
)
```

Aliases: `dvs_delete`, `dvs_remove`, `dvs_rm`.

- `files`: list of files that are to be deleted.

### Non-existing files

Emit a warning, but still remove the files that do exist and are tracked.
