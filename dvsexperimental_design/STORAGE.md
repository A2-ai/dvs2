# Storage layout and object lifecycle

This document describes where DVS stores data, how objects are addressed, and
how files move between working tree, external storage, local cache, and remote
CAS.

## Storage layers

DVS intentionally separates data from Git. There are three distinct storage
layers:

1) Working tree (your repo)
   - The data files you edit.
   - Metadata files (`.dvs` / `.dvs.toml`) live next to data files and are
     committed to Git.

2) External storage directory
   - Configured in the repo config (`dvs.yaml` / `dvs.toml` / `dvs.json`).
   - Content-addressed object storage for large data.
   - Typically on a shared filesystem (NAS, shared drive, mounted storage).

3) Local cache (`.dvs/cache/objects`)
   - A per-repo cache used for remote push/pull and materialization.
   - Stores objects by hash, same layout as external storage.

There is also an optional remote HTTP CAS server that mirrors the same object
layout and protocol for push/pull.

## Content addressing and OIDs

DVS names objects by content hash. The object identifier (OID) includes the
algorithm:

- Format: `algo:hex`
- Examples: `blake3:...`, `sha256:...`, `xxh3:...`

The storage layout uses the algorithm and a two-character prefix for fan-out:

```
{root}/{algo}/{first_two_chars}/{remaining_chars}
```

For example:

```
/storage/blake3/ab/c123def...
.dvs/cache/objects/sha256/ff/0011aa...
```

This layout prevents huge single directories and supports multiple algorithms
side-by-side without collisions.

## External storage directory

External storage is configured in the repo config:

- `storage_dir`: required absolute or relative path
- `permissions`: optional Unix permissions for stored objects
- `group`: optional Unix group for stored objects

When `dvs add` runs, it hashes the file and copies it into the external
storage directory using the layout above. If the object already exists, the
copy is skipped (content-addressable, immutable).

Notes:
- Copying is buffered but not atomic (current implementation writes directly to
  the destination). If atomicity is required, add a temp + rename strategy.
- Permissions/group are applied only on Unix and only at copy time.

## Local cache (`.dvs/cache/objects`)

The local cache mirrors the external storage layout under `.dvs/cache/objects`.
This cache is used by:

- `dvs pull` (downloads objects into the cache)
- `dvs push` (uploads objects from the cache)
- `dvs materialize` (copies cached objects into the working tree)

The cache is distinct from external storage. Today, `dvs add` writes directly
to `storage_dir` and does not populate `.dvs/cache/objects`. This means `dvs
push` can fail if the object is not already cached. The sync-first roadmap
calls out this mismatch for remediation.

## Manifest, metadata, and storage

DVS has two layers of tracking:

- Metadata (`file.ext.dvs` / `file.ext.dvs.toml`): per-file record tracked in
  Git. This is what `status` compares against.
- Manifest (`dvs.lock`): a repo-wide list of path -> OID mappings, used for
  push/pull and materialize. The manifest is also tracked in Git.

Current behavior:
- `dvs add` updates metadata and also inserts/updates the manifest entry.
- `dvs pull`/`push`/`materialize` use the manifest as source-of-truth.

Target behavior (sync-first): the manifest is always in sync with metadata, and
`sync` orchestrates cache pull + materialize + push in one command.

## Object lifecycle (current)

### Add (local publish)

1) Compute hash for each file.
2) Copy file into external storage (`storage_dir`).
3) Write metadata next to the file.
4) Update manifest (`dvs.lock`).
5) Update reflog snapshot.

### Get (local restore from external storage)

1) Read metadata file.
2) Compare local hash.
3) If missing or mismatched, copy object from `storage_dir` to the working tree.

### Pull (remote -> cache)

1) Read manifest (`dvs.lock`).
2) Resolve remote URL (CLI arg, local config, or manifest base_url).
3) Download each missing object into `.dvs/cache/objects`.

### Materialize (cache -> working tree)

1) Read manifest (`dvs.lock`).
2) For each entry, if cached and not already materialized, copy from cache into
   working tree.
3) Update `.dvs/state/materialized.json` to avoid redundant writes.

### Push (cache -> remote)

1) Read manifest (`dvs.lock`).
2) Resolve remote URL.
3) For each object, upload from `.dvs/cache/objects` to remote.

## Garbage collection (not implemented)

There is no GC at the moment. A safe GC design would:

- Collect all OIDs referenced by metadata and/or manifest.
- Remove objects from external storage and cache that are unreferenced.
- Keep objects referenced by reflog snapshots if rollback is supported.

## Known gaps and inconsistencies

- `dvs add` writes to external storage but does not populate the local cache.
  `dvs push` expects the object in the cache.
- `dvs init` currently adds `*.dvs` and `*.dvs.toml` to `.gitignore`, which
  conflicts with the goal of committing metadata.
- Manifest and metadata can drift if operations write one but not the other.

See ROADMAP.md and plans/048-cli-sync.md for the intended fixes.
