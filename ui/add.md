# `dvs add`

Goal: Add files to an initialised dvs repository.

## CLI

The option to return `--json` must be present.

## R

Signature:

```r
dvs_add <- function(path = ".", 
  files = character(), 
  glob = character(),
  ignore.case = NULL %||% !is.empty(glob),
  overwrite = FALSE,
  fail = FALSE
)
```

- `path` is location of dvs repository; the `dvs.toml` has to be present
  in an ancestor to `path`.

## Compression

If the added file exceeds a certain threshold, the
R package should provide suggest compressing the recently added file.

- [ ] `getOption(dvs.large_file_size = integer()`)
