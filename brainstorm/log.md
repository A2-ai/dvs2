# `dvs log`

Per file logging is inspected via `dvs log` / `dvs_log()`. For project-wide logging, we have `dvs audit` / `dvs_audit()`.

## CLI

The option to return `--json` must be present.

```sh
# in a previously `dvs init` folder
$ dvs log data/derived/model_summary.txt
Last edited on: 20-10-2020
checksum: NNNNN
message: "Ran nonmem model on exposure assumptions" 
```

```sh
$ dvs log --interval <duration>
[date -- duration since now]
Last edited on: 20-10-2020
checksum: NNNNN
message: "Ran nonmem model on exposure assumptions" 

Last edited on: 20-10-2020
checksum: NNNNN
message: "Ran nonmem model on exposure assumptions" 

Last edited on: 20-10-2020
checksum: NNNNN
message: "Ran nonmem model on exposure assumptions" 
[date -- 2x duration since now]
...
[date -- 3x duration since now]
...
```

`<duration>`: `days`, `weeks`, `months`

## R

Signature:

```r
dvs_log <- function(
  since = NULL,
  by_user = NULL,
)
```


