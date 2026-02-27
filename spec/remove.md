# dvs remove / `dvs remove` / `dvs::dvs_remove`

Goal: Provide a way to untrack a file without deleting it from the project.

## CLI

```sh
dvs -- Remove files from dvs tracking

Usage:
    dvs remove <FILES> [OPTIONS]
    dvs rm <FILES> [OPTIONS] # alias
 
Files:
    Paths to tracked files that must be untracked

Options:
    --json Command output as a JSON format
    -m, --message (optional) message describing why a file was removed from tracking
    -h, --help
```

Files that are not tracked, but provided to `dvs remove` must result in an error. Deletion is a sensitive operation.

## R

Functionally an alias to `dvs_delete` by

```r
dvs_remove <- function(...) {
    dvs_delete(delete_cached=FALSE, ...)
}
# alias
dvs_untrack <- dvs_remove
```
