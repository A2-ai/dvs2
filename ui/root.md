# `dvs root`

Goal: return the location of the dvs repository root anywhere.

## CLI

Not relevant.

## R package

Signature:

```r
# note: no `path` parameter, always assume current directory
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
