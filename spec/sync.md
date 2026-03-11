# `dvs sync`

**Status: not implemented as a separate command.** The same result can be achieved with `dvs get --glob "**/*"`.

Retrieves all tracked files in the project from storage. Intended as a convenience command for pulling all data after cloning a repository.

## Proposed behavior

- Equivalent to `dvs get --glob "**/*"` run from the project root.
- Retrieves every tracked file regardless of the user's current directory within the project.
- Follows the same outcome semantics as `get`: files already matching metadata are skipped (`present`), others are copied (`copied`).

## Proposed CLI

```
dvs sync [OPTIONS]

Options:
      --json       Output as JSON
      --dry-run    Show what would be retrieved without making changes
  -h, --help       Print help
```

No path or glob arguments. Always operates on the entire project.

### Exit codes

- `0`: all files retrieved successfully.
- `1`: one or more files failed.

## Proposed R package

```r
dvs_sync()
```

Thin wrapper around `dvs_get` with a project-wide glob.
