# `dvs add`

Goal: Add files to an initialized dvs repository.

- [ ] Currently the `message` is attached to all files checked in simultaneously.
  dvs has a log and audit log to
  illuminate "why" a change occurred in the data.

<!-- TODO: how do we enhance the why illumination? -->

## CLI

- Assume that current directory is a dvs repository, both in cli and R-package.
- The option to return `--json` must be present.

## R

Signature:

```r
dvs_add <- function(
  files = character(), 
  glob = character(),
  ignore.case = NULL %||% !is.empty(glob),
  overwrite = FALSE,
  fail = FALSE
)
```



## Compression

If the added file exceeds a certain threshold, the
R package should provide suggest compressing the recently added file.

- [ ] `getOption(dvs.large_file_size = integer()`)
  - Hard limit 100 MB [PMx-project-template](https://github.com/A2-ai/template-PMx-project-starter/blob/main/.lefthook/pre-commit/file-size)
  - Soft limit 50 MB (warning emitted) [PMx-project-template](https://github.com/A2-ai/template-PMx-project-starter/blob/main/.lefthook/pre-commit/file-size)
