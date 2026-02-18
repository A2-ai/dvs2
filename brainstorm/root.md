# `dvs root`

Convenience utility for expert users

Goal: Return the location of the dvs repository root anywhere.

## CLI

Not relevant.

## R package

Signature:

```r
dvs_root <- function(...)
# alias
find_dvs_root <- dvs_root()
```

Convenience:

```r
dvs_root("model_code")
# equivalent to
fs::join(dvs_root(), "model_code")
# or
file.path(dvs_root(), "model_code")
```

The use cases for this function is very limited. We assume heavy use of
`{here}`-package in dvs-based projects. But it could be a relevant convenience
function in certain, specific cases.
