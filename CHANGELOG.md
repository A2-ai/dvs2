# Changelog

## Unreleased

### Breaking

- R package: requires R 4.4 or later.

### Added

- R package: `just rpkg-build-tarball` builds the release source tarball in `dist/`. It carries every Rust crate, so it installs offline. `just rpkg-check-cran` runs `R CMD check --as-cran` on it.
- R package: `just rpkg-reference` builds the HTML reference manual in `dist/reference/`.

### Fixed

- R package: `dvs_add()` and `dvs_get()` with a progress bar could fail with "Unknown cli progress bar configuration", when R collected garbage before the bar started.

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
