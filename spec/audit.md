# `dvs audit`

Goal: Provide a repository wide log of dvs tracked files.

While the command `dvs audit` is on a repository, the audit log resides on the backend, and can contain miscellaneous information.

## CLI

The option to return `--json` must be present.

```sh
dvs audit log

Usage:
    dvs audit [PATHS] [OPTIONS]

Paths:
    Optional paths to files and directories for which to retrieve dvs repository events. 

Options:
    --glob              glob expression describing which files and folders to retrieve dvs events for
    --users=[users..]   list of users that are responsible for dvs events
    --filter-added      retrieve audit log of the added files
    --filter-deleted    retrieve audit log of the deleted files
    --filter-updated    retrieve audit log of the updated (added multiple times) files
    --events=<events..>  which dvs events to retrieve
    --invert            invert the selection in `--filter-*`/`--events`
    --since             retrieve dvs events since this date.
    -h, --help

```

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
dvs_audit <- function(
    paths = c(),    # optional, filters the audit log to the files/directories shown
    glob = c(),     # optional, glob expression representing `paths`
    users = c(),    # optional, list of users that caused dvs events of interest
    filter = c(),   # optional, which dvs-event should be shown?
    invert = FALSE, # opposite of selection in `filter`
)
```

```r
dvs_audit()
```

```r
dvs_audit(since = NULL)
```
