# Status and sync policy

This document defines how DVS determines discrepancies and how the sync-first
workflow should resolve them. It also documents the tradeoffs of auto-adding
modified files.

## Status model

Today, `dvs status` compares the working tree to metadata and returns a
per-file status:

- `Current`: local file matches metadata hash
- `Absent`: metadata exists, local file missing
- `Unsynced`: local file exists but hash differs
- `Error`: unable to determine status

Target status model expands this to include cache and remote availability:

- Local vs metadata (Current / Absent / Unsynced)
- Cache presence (object exists in `.dvs/cache/objects`)
- Remote presence (object exists on HTTP CAS)
- Manifest vs metadata consistency (dvs.lock matches .dvs files)

Status should be able to answer: "What would sync do?" without changing data.

## Materialize: why it exists

`materialize` copies cached objects into the working tree. It is needed because
DVS can fetch objects into the cache without overwriting local files. This
enables:

- Partial restores (only a subset of files)
- Safe updates without clobbering dirty local files
- Offline usage once objects are cached

In a sync-first CLI, materialize is mostly an internal step, but it remains
useful as an explicit recovery tool.

## Sync policy (target)

The sync-first command should do three things in order:

1) Pull: fetch missing objects from remote into the local cache.
2) Materialize: copy cached objects into the working tree when safe.
3) Push: upload new objects to remote (push-by-default).

### Auto-add policy

Default: **no auto-add of dirty tracked files**.

Optional: `dvs sync --update` (or `--include-dirty`) to add dirty tracked files
and update metadata/manifest. When `--update` is used:

- Require `--message` or auto-generate one (timestamp + file list).
- Only operate on files that already have metadata (tracked files).
- Do not auto-add untracked data files.

This keeps DVS safe and explicit by default while allowing a convenience path
when you opt in.

## Consequences of auto-adding tracked files

If sync auto-adds by default, the system becomes optimistic about local state
being the source of truth. The risks are significant:

- **Accidental publication**: temporary or partial edits can be pushed to
  remote storage without an explicit publish step.
- **Surprise overwrites**: a local file could silently replace a newer remote
  version, especially in multi-user environments.
- **Harder debugging**: unintended versions appear in history without clear
  intent or message.
- **Automation hazards**: background processes or notebooks can change data
  files and trigger unintended versioning during sync.

If sync does **not** auto-add by default:

- **Safer defaults**: users must explicitly add versions they intend to share.
- **Clear intent**: changes only enter history when the user chooses.
- **More manual steps**: users must remember to run `add` before `sync`.

Given the project's goal to minimize conflicts and surprise writes, the
recommended default is **no auto-add** with an explicit `--update` flag.

## Conflict resolution (future)

DVS is designed around "latest wins" rather than complex merges. However, the
current code does not yet define how "latest" is determined across local and
remote. A complete design should:

- Track provenance (who/when) in metadata and/or manifest.
- Compare versions across local and remote before overwriting.
- Provide clear reporting in `status` and `sync` when versions differ.

Until that exists, `sync` should avoid overwriting dirty local files without
explicit user intent.
