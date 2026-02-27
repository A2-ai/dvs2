# dvs remove / `dvs remove` / `dvs::dvs_remove`

Goal: Provide a way to untrack a file without deleting it from the project.

## R

Functionally an alias to `dvs_delete` by

```r
dvs_remove <- function(...) {
    dvs_delete(delete_cached=FALSE, ...)
}
# alias
dvs_untrack <- dvs_remove
```
