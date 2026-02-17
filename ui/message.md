# `dvs_message`

Goal: Add messages to files without re-hashing or replacing them.

## CLI

```sh
$ dvs message data/model_aaabb/model_summary.csv "this time it was run with 10000 repititons"
Added message to `data/model_aaabb/model_summary.csv`
```

## R package

```r
dvs_message <- function(path = ".", 
  files = character(), 
  glob = character(),
  ignore.case = NULL %||% !is.empty(glob),
  fail = FALSE
)
```

`dvs_message` is a equivalent to an idempotent `dvs_add`-call.
