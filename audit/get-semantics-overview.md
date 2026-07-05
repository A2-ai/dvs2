# `dvs get` semantics

Based on `origin/main` at `040aa31`. Verified by reading the resolution code
(`dvs/src/paths.rs`, `dvs/src/globbing.rs`, `dvs/src/files/get.rs`,
`dvs-cli/src/main.rs`) and by an empirical R run against that code (nested-cwd
recursive get, see below).

## The model

`get` resolves your arguments into a set of tracked files, then copies each
from storage to its path in the project and verifies the hash. Everything
below is about that resolution step plus what happens per file.

## Two things that surprise people first

### 1. The CLI requires a path or `--glob`. The R/library API does not.

- CLI: `paths` is `required_unless_present = "glob"` (`dvs-cli/src/main.rs:88`).
  Bare `dvs get` is a clap parse error. To restore everything you must type a
  path, `.`, or `--glob '**/*'`.
- R: `dvs_get(paths = character(0))` is the default and is allowed. Empty paths
  scope to the working directory.
- Library: `resolve_paths_for_get(vec![], ...)` is allowed and scopes to cwd.

This asymmetry is real, not a doc mistake. The CLI is stricter than the
library it wraps.

### 2. "No paths" and `recursive` are scoped to the current working directory, not the repo root.

The no-paths filter (`PathFilter::cwd_scoped`, `dvs/src/paths.rs:255`) uses
`cwd_relative_to_root()` as a prefix filter, so only tracked files under your
cwd match. Confirmed empirically: from a nested `data/` folder,
`dvs_get(recursive = TRUE)` restored only `data/raw/deep.csv` and left the
repo-root `top.csv` untouched.

## Path arguments

| Argument | Resolves to |
|----------|-------------|
| a tracked file | that file |
| a directory | tracked files directly under it (with `recursive`: every depth under it) |
| `.` | the cwd (so `dvs get .` = direct children of cwd, `dvs get . -r` = cwd and down) |
| absolute path | stripped to repo-root-relative, then as above (`from_user_paths`, `dvs/src/paths.rs:268`) |
| several paths | union of all of them |

Every explicit path must match at least one tracked file. If any matches
nothing (never added, misspelled), the whole batch is refused and nothing is
retrieved (`dvs/src/globbing.rs:130`, "Bail on any invalid path" #240).

## No paths (R/library only)

| Call (cwd = some folder) | Restores |
|--------------------------|----------|
| `dvs_get()` | tracked files directly under cwd |
| `dvs_get(recursive = TRUE)` | every tracked file under cwd, any depth |

At the repo root, cwd-relative is empty, so non-recursive = root's direct
children and recursive = the entire repo.

## glob

- Matched against tracked paths relative to cwd (or relative to each path
  argument when paths are given). See `matches` at `dvs/src/paths.rs:305`.
- `--glob '*.csv'` matches only the current level. Subdirectories need `**`:
  `--glob '**/*.csv'`.
- `--glob '**/*'` = everything under cwd (everything in the repo if at root).
- Mutually exclusive with `--recursive` (`conflicts_with`, plus an explicit
  error in the R wrapper).

## recursive

Only affects how directories expand (explicit dir arguments, or the implicit
cwd when no paths). Non-recursive = direct children, recursive = all depths. It
does nothing to a file argument and nothing in combination with `--glob`.

## Per-file outcome

- `copied`: retrieved from storage.
- `present`: local file already matches the metadata, no-op.
- After copy, the hash is re-verified against metadata. On mismatch the
  retrieved file is deleted and that file is marked failed.

## Errors and exit codes

- Untracked or misspelled path in the request: whole batch refused, nothing
  retrieved, exit `1` (CLI) or error raised (R).
- Glob or no-paths resolution that matches nothing: error.
- A file that fails after a valid batch starts (hash mismatch, missing in
  storage): that file fails, the rest still retrieve, exit `1` (CLI). In R it
  is one result row with the `error` column set and success columns `NA`, no
  exception.
- The Rust library returns results sorted by path and leaves per-file failure
  handling to the caller.

## dry_run

Reports what would be retrieved, makes no changes.

## Quick reference

| You run | From repo root | From nested `data/` |
|---------|----------------|---------------------|
| `dvs_get()` (R) | direct children of root | direct children of `data/` |
| `dvs_get(recursive = TRUE)` (R) | whole repo | `data/` and below only |
| `dvs get` (CLI) | error: needs path or glob | error: needs path or glob |
| `dvs get .` | direct children of root | direct children of `data/` |
| `dvs get . -r` | whole repo | `data/` and below only |
| `dvs get --glob '**/*'` | whole repo | `data/` and below only |
| `dvs get /path/to/root -r` | whole repo | whole repo |

## Related

Spec PR #248 (`docs/specs-get-no-paths-recursive`) corrects the `get` section
of `specs.md` to match this. The spec, the Rust doc comment on `dvs_get`
(`dvs-rpkg/src/rust/lib.rs:423-426`), the core globbing unit tests
(`get_no_paths_non_recursive_returns_direct_children`,
`get_no_paths_recursive_returns_all_under_cwd`), and this overview are aligned.
