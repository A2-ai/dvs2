# `dvs audit`

Goal: Provide a repository wide log of dvs tracked files.

## CLI

The option to return `--json` must be present.

```sh
$ dvs audit
[Date] [User] [+{files} -{files}] [Message]
```

```sh
$ dvs audit --since <date|duration>
```

## R

Signature:

```r
dvs_audit <- function()
```

```r
dvs_audit()
```

```r
dvs_audit(since = NULL)
```
