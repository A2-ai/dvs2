# Changelog

## Unreleased

### Added

- `dvs migrate`: converts a checkout to newer storage, leaving the old storage untouched. `dvs migrate local` moves a dvs1 project to local storage (its dvs1 folder in place, or a copy at a path), `dvs migrate server` moves a dvs1 or local project to a dvs server, and a bare `dvs migrate` copies what a migrated checkout still lacks, for example after merging an unmigrated branch. It never commits: it lists the changed files to commit.
- `dvs get` and `dvs status` read dvs1 checkouts (`dvs.yaml`). `dvs add` refuses them and suggests `dvs migrate`.

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
