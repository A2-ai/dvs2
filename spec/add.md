# `dvs add`

Goal: Add files to an initialized dvs repository.

## CLI

- Assume that current directory is a dvs repository, both in cli and R-package.
- The option to return `--json` must be present.

## R

Signature:

```r
dvs_add <- function(
  files = character(), 
  glob = character(),
)
```

## Compression

If the added file exceeds a certain threshold, the
R package should provide suggest compressing the recently added file.

- [ ] `getOption(dvs.large_file_size = integer()`)
  - Hard limit 100 MB [PMx-project-template](https://github.com/A2-ai/template-PMx-project-starter/blob/main/.lefthook/pre-commit/file-size)
  - Soft limit 50 MB (warning emitted) [PMx-project-template](https://github.com/A2-ai/template-PMx-project-starter/blob/main/.lefthook/pre-commit/file-size)

Advice compression when

- a single size exceeds size thresholds
- a directory of files exceeds size thresholds

There are cases where individual files are not large, but the collection of files
starts to amount to a large amount, presumably too large to track.
