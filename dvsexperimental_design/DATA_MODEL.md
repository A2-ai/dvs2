# Data model

This document describes the core data types and on-disk formats used by DVS.

## Repo config (dvs.toml / dvs.yaml / dvs.json)

Stored at repo root. Defines storage location and defaults.

Fields (current):
- storage_dir: path to external storage directory
- permissions: optional file permissions for stored objects
- group: optional group for stored objects
- hash_algo: optional default hash algorithm (blake3, sha256, xxh3)
- metadata_format: json or toml
- generated_by: version/commit info

File name depends on feature flags:
- default: dvs.toml
- yaml-config feature: dvs.yaml

## Metadata files (.dvs / .dvs.toml)

Stored next to data files and committed to Git.

Fields:
- blake3_checksum (string): historical field name for the content hash
- size (u64)
- add_time (timestamp)
- message (string)
- saved_by (string)
- hash_algo (enum) - indicates which algorithm produced the checksum

Format:
- JSON: file.ext.dvs
- TOML: file.ext.dvs.toml

TOML is preferred when both exist.

## Manifest (dvs.lock)

Tracks which objects belong to the repo for remote sync.

Fields:
- version (schema version)
- base_url (optional default remote)
- entries[]:
  - path (repo-relative)
  - oid (algo:hex)
  - bytes (uncompressed size)
  - compression (none, zstd, gzip, lz4)
  - remote (default "origin")

The manifest is the remote sync source of truth.

## Oid

Object identifier that includes algorithm and hex:

- format: algo:hex
- examples: blake3:..., sha256:..., xxh3:...

Used for storage paths and server endpoints.

## Local config (.dvs/config.toml)

User-specific settings not committed to Git.

Fields:
- base_url: default remote URL for push/pull/sync
- auth.token: bearer token for HTTP CAS
- cache.max_size: optional cache limit

## Local cache state (.dvs/state/materialized.json)

Tracks which files were materialized to avoid redundant writes.

Fields:
- files: map of path -> oid
- last_materialized: timestamp

## Reflog and snapshots

Reflog tracks metadata history and rollback.

- .dvs/refs/HEAD: current state id
- .dvs/logs/refs/HEAD: JSONL reflog entries
- .dvs/state/snapshots/{id}.json: WorkspaceState snapshots

Reflog entry fields:
- ts, actor, op, message, old, new, paths

WorkspaceState:
- version
- manifest (optional)
- metadata entries (path + metadata)

## Status and outcomes

Status values (FileStatus):
- current: local matches metadata
- absent: metadata exists, local missing
- unsynced: local differs from metadata
- error: unable to determine

Outcomes (Outcome):
- copied
- present
- error
