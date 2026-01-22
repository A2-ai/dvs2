# Workflows

This document describes the expected end-to-end workflows for DVS. It separates
current behavior from the sync-first target where daily usage is limited to
`add`, `sync`, and `status`.

## 1) Initialize a repository

Current behavior:

1) `dvs init <storage_dir>`
2) Creates the repo config (`dvs.yaml` / `dvs.toml` / `dvs.json`).
3) Creates or validates the external storage directory.
4) Adds ignore patterns to `.gitignore` (currently `*.dvs` and `*.dvs.toml`).

Target behavior:
- The ignore patterns should protect large data files, not metadata.
- Metadata files should be committed to Git.

## 2) Add data

Current behavior:

1) `dvs add <files...>`
2) Hash each file with the configured algorithm.
3) Copy into external storage.
4) Write metadata next to each file.
5) Update manifest (`dvs.lock`).
6) Record reflog snapshot.

Notes:
- `add` is an explicit publish step. It does not auto-detect dirty files.
- If the file already exists with the same hash, the add is a no-op.

Target behavior:
- `add` remains explicit.
- `sync --update` can optionally stage dirty tracked files as a convenience.

## 3) Sync (target workflow)

The goal is a single command that converges local and remote state and minimizes
push/pull mental overhead.

Target flow for `dvs sync`:

1) Pull: download missing objects from remote into `.dvs/cache/objects`.
2) Materialize: copy cached objects into the working tree.
3) Push: upload local objects to remote (push-by-default).

Policy decisions (from plans/048-cli-sync.md):
- Default: no auto-add of dirty tracked files.
- Optional flag: `--update` or `--include-dirty` to add dirty tracked files.
- When `--update` is used, require a message or auto-generate one.

This preserves explicit intent for new versions while still enabling a
one-command flow.

## 4) Status

Current behavior:

- `dvs status` compares the working tree against metadata (`.dvs` files).
- Results are per-file: Current, Absent, Unsynced, Error.

Target behavior:

- Status should also show cache/remote availability and what sync would do.
- It should explicitly report:
  - Local matches metadata
  - Local missing
  - Local diverged from metadata
  - Object missing from cache or remote

## 5) Fetch or restore files

Current behavior has two paths:

- `dvs get` restores from external storage (configured `storage_dir`).
- `dvs pull` downloads from remote CAS into local cache.
- `dvs materialize` copies from cache into working tree.

Target behavior:

- `dvs sync` should cover the common path (pull + materialize) and replace
  `get`/`pull` for daily usage.
- `materialize` remains for advanced/partial restores and offline use.

## 6) Remote publish

Current behavior:

- `dvs push` uploads objects from `.dvs/cache/objects` to a remote CAS server.
- `dvs pull` downloads from that remote.

Target behavior:

- `sync` replaces `push`/`pull`.
- `push`/`pull` become deprecated or removed.

## 7) Rollback and audit

Current behavior:

- `dvs log` shows reflog entries.
- `dvs rollback <id>` restores metadata (and optionally materializes data).

Target behavior:

- Keep rollback as an advanced tool.
- Ensure reflog snapshots include manifest and metadata in a consistent format.

## 8) Merge repositories

Current behavior:

- `dvs merge-repo` imports metadata + objects from another DVS repo.

Target behavior:

- Keep as a migration tool (rare usage).
- Ensure manifest and metadata are reconciled during import.

## 9) R interface

There is an `dvsR` crate for R bindings, intended to expose a minimal API
mirroring the CLI. The objective is a small set of functions that map to the
sync-first workflow:

- init
- add
- sync
- status

This is not fully implemented yet.

## 10) Daemon (future)

The daemon is planned to:

- Watch for file changes.
- Optionally auto-add or auto-sync.
- Expose an IPC API for UI integrations.

This is not implemented today.
