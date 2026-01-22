# Glossary

- **CAS (Content-Addressable Storage)**: Storage addressed by content hash
  rather than filename.

- **OID (Object ID)**: The hash identifier for an object. Format: `algo:hex`.

- **External storage**: The shared directory configured by `storage_dir` where
  DVS stores objects.

- **Local cache**: The `.dvs/cache/objects` directory used for pull/push and
  materialize.

- **Metadata file**: The `.dvs` or `.dvs.toml` file next to a data file. Tracked
  in Git; records hash, size, timestamp, author, and message.

- **Manifest (`dvs.lock`)**: Repo-wide list of tracked paths and OIDs used for
  sync with remote storage.

- **Materialize**: Copy cached objects into the working tree.

- **Status**: A comparison between local files and metadata (and, in the
  target design, cache/remote availability).

- **Sync**: The planned single command that pulls, materializes, and pushes to
  converge local and remote state.

- **Reflog**: History of metadata state snapshots for rollback/audit.

- **Workspace state**: A snapshot of tracked metadata (and eventually manifest)
  used by reflog.

- **Remote CAS**: The HTTP server (`dvs-server`) that stores objects by OID.
