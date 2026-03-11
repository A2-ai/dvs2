# `dvs remove`

**Status: not implemented.** This spec describes proposed behavior.

Untracks a file from DVS without deleting it from the project or from storage.

## Proposed behavior

- Deletes the metadata sidecar file (`.dvs` file).
- Does not delete the local data file.
- Does not delete the blob from backend storage. Other files or projects may reference the same content-addressed blob.
- Removes the file's entry from `.gitignore`.
- Files that are not tracked produce an error.
- Best-effort: if some files fail, the rest are still processed.

This is functionally equivalent to `dvs delete --cached` without removing from storage.

## Proposed CLI

```
dvs remove [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...              Tracked files to untrack

Options:
  -m, --message <MESSAGE> Optional message explaining why the file was removed
      --json              Output as JSON
  -h, --help              Print help
```

`dvs rm` is an alias for `dvs remove`.

### Exit codes

- `0`: all files untracked successfully.
- `1`: one or more files failed.

## Proposed R package

```r
dvs_remove(
  files,
  message = NULL
)
```

Alias: `dvs_untrack`.
