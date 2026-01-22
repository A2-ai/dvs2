# Architecture

## High level components

DVS is split into layers. The core library is pure Rust with file operations,
while the CLI and server are thin wrappers.

```
+-------------------+          +---------------------+
|      dvs-cli      |          |     dvs-server      |
|  user commands    |          |  HTTP CAS endpoints |
+---------+---------+          +----------+----------+
          |                               |
          |                               |
          v                               v
+----------------------------------------------------+
|                    dvs-core                        |
| config, metadata, hashing, storage, operations     |
+---------+---------------------------+--------------+
          |                           |
          |                           |
          v                           v
+-------------------+          +---------------------+
| External storage  |          |   .dvs local cache  |
| content-addressed |          |  objects + state    |
+-------------------+          +---------------------+

Optional / future layers:
- dvs-daemon: background watcher and auto-sync
- dvsR: R bindings (currently minimal)
- dvs-testkit: conformance tests
```

## Crate responsibilities

- dvs-core
  - Data model: Config, Metadata, Manifest, Oid, Reflog
  - Storage: external storage paths, local cache paths
  - Operations: init, add, get, status, push, pull, materialize, log, rollback
  - Helpers: hashing, copy, ignore, layout, reflog

- dvs-cli
  - Argument parsing and human output
  - Delegates to dvs-core
  - Houses install/uninstall for completions and git-status shim

- dvs-server
  - HTTP CAS endpoints: HEAD/GET/PUT/DELETE objects
  - Auth and permissions
  - Local storage backend for object files

- dvs-daemon (future)
  - File watcher, IPC, auto-add/sync

- dvs-testkit
  - Cross-interface tests and scenarios

- dvsR (future)
  - R bindings for DVS operations

## Data flow: add

```
User writes file -> dvs add
  -> hash file
  -> copy to external storage
  -> write metadata file next to data file
  -> update .gitignore
  -> update reflog snapshot
```

## Data flow: sync (target)

```
User runs dvs sync
  -> pull missing objects into .dvs/cache
  -> materialize cache objects into working tree
  -> push new objects to remote
  -> (optional) update metadata if --update is used
```

## Key directories

- repo root
  - dvs.toml / dvs.yaml (config)
  - dvs.lock (manifest)
  - data files + .dvs metadata
  - .dvs/ (local cache and state)

- external storage
  - content-addressed objects by hash

## Known gaps vs target design

- Manifest is not consistently written by add (sync relies on it)
- Metadata format handling is partial (.dvs.toml vs .dvs)
- Storage layout is inconsistent (hash-only vs algo-prefixed)
- Server integrity checks do not verify payload hash

See ROADMAP.md for remediation.
