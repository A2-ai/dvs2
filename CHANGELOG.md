# Changelog

## Unreleased

### Breaking

- `dvs add` with a path that does not exist is `invalid_arguments` and exits 2, as a directory without `--glob` and a path outside the project are. It was `no_files_selected` with exit 1. The refusal names every invalid path, sorted, with its reason. It stopped at the first missing path. `dvs_add()` raises the same message.
- R package: requires R 4.4 or later.
- R package: `dvs_audit_log()` spreads each entry's action into columns: `action_variant` ("init" or "add") and one `action_<field>` column per field that the actions carry, `NA` where an action has none. The action was a list-column. `timestamp` is a date-time (UTC), not unix seconds.
- R package: `NA` is never a value. An `NA` in any argument raises an error that says it must not be `NA`, also among several values, as in `on_error = c("return", NA)`. An `NA` `message`, `group`, `root_dir` or `metadata_folder_name` meant the argument was not given. Use `NULL` or leave the argument out.
- R package: `dvs_add()` with neither `paths` nor `glob` is an error, as in the CLI.
- R package: `new_dvs_bytes(NULL)` is an error. `numeric(0)` still gives an empty vector.
- R package: `format_byte_size(TRUE)` and `format_byte_size(FALSE)` are errors, as `set_dvs_threads(TRUE)` is. They returned `"1 B"` and `"0 B"`.

### Added

- R package: `just rpkg-build-tarball` builds the release source tarball in `dist/`. It carries every Rust crate, so it installs offline. `just rpkg-check-cran` runs `R CMD check --as-cran` on it.
- R package: `just rpkg-reference` builds the HTML reference manual. The one in `dist/reference/` is for users and leaves out the internal pages, as R's help index does. The one in `dvs-rpkg/rust-target/doc/r/` is for development and keeps them.
- R package: `dvs_status()` has a `code` column, the kind of failure of a row that failed, as in the CLI's JSON.
- R package: `dvs_get()` has a `duration_ms` column.
- R package: the counts of `dvs_result_info()` include `total`, `succeeded` and `cancelled`, as in the CLI's JSON.
- R package: a wrong argument raises a `dvs_error_argument` condition, also a `dvs_error`, with the argument in `param` and the call the user wrote. A `dvs.toml` that does not load raises a `dvs_config_error`, also a `dvs_error`, in every command. One `dvs_error` handler catches them all.
- R package: a bad `dvs.num_threads` or `dvs.on_error` option raises the same `dvs_error_argument`, and the message names the option.
- R package: `dvs_result_info()` has `summary`, the summary the CLI prints. A printed `dvs_get()` result and its `on_error` warning or error start with it, so R and the CLI say the same. `on_conflict = <policy> (<source>)` appears only when the policy acted, as in the CLI.

### Fixed

- R package: `dvs_add()` and `dvs_get()` with a progress bar could fail with "Unknown cli progress bar configuration", when R collected garbage before the bar started.
- R package: `dvs_status()` had no `message` column when the first file had no message. It now returns the same columns for every repository, also one with no tracked files.
- R package: `dvs_status()` with a repeated `status` choice could show every status.
- R package: `NULL` in an optional argument means the default, as leaving it out does. `paths = NULL` raised an error, and `dvs_get(on_error = NULL)` skipped the `dvs.on_error` option.
- R package: `character(0)` in `paths` or `status` means the default. `dvs_status(status = character(0))` raised an error.
- R package: `dvs_status(paths = NA)` reported on the working directory.

## 0.5.1

### Fixed

- `dvs info` no longer reports the default compression (`zstd`) for a server project. The server sets it, so `info` now says "set by the server", and `--json` reports `compression: null`.

## 0.5.0

### Breaking

- `dvs init` takes a storage backend: `dvs init <path>` is now `dvs init local <path>`. The old form exits with an error pointing at the new one.
- `dvs get` no longer replaces files with local changes. It aborts by default. Choose what happens with `--on-conflict abort|skip|overwrite`, or with `--force`.

### Added

- Server backend: store data on a dvs server with `dvs init server`, join an existing server project with `dvs attach`, and use `dvs login` and `dvs logout`.
- New commands:
  - `dvs info`: shows the project's configuration.
  - `dvs history`: shows a file's upload history.
  - `dvs availability`: checks whether content is stored.
- Versioned `--json` output and `--emit-events` progress streaming.

## 0.4.0

A modern rewrite of the original [A2-ai/dvs](https://github.com/A2-ai/dvs) R package. It has a Rust core with a `dvs` CLI and an R package on top.
