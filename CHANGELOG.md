# Changelog

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
