# Commands and why they exist

This doc explains each command and why it is necessary. The design intent is a
sync-first workflow where daily usage is limited to `add`, `sync`, and `status`.

## Daily commands

### dvs init

Purpose: One-time setup to connect the repo to external storage and create config.

Why it exists: DVS separates storage from tracking. The repo must know where the
external storage directory lives. This is a setup step, not daily usage.

---

### dvs add <files...>

Purpose: Publish a new data version.

Why it exists: DVS only tracks metadata in Git. `add` is the explicit action that:
- hashes the file (content address)
- copies the file to external storage
- writes metadata next to the file
- updates .gitignore to avoid committing data files

Without `add`, sync has nothing authoritative to share.

---

### dvs sync [files...]

Purpose: Converge local and remote state (push by default).

Why it exists: The Git-like split between push/pull/get is not useful for data.
Sync should handle the normal path end-to-end:

1) pull missing objects
2) materialize files
3) push new objects

If local is dirty, sync does not auto-add by default (see STATUS_AND_SYNC.md).

---

### dvs status [files...]

Purpose: Report discrepancies between local, metadata, and remote.

Why it exists: DVS decouples storage and tracking. Status is how you know if:
- local files are missing
- local files diverged from metadata
- cache/remote has missing objects

Target behavior is to show local vs manifest and local vs remote availability.

## Advanced or rare commands

### dvs materialize

Purpose: Copy cached objects from .dvs/cache into the working tree.

Why it exists: DVS may cache objects locally without overwriting files. This
command allows explicit restoration or partial materialization. In a sync-first
workflow it is usually an internal step.

---

### dvs log / dvs rollback

Purpose: Audit and restore metadata history (reflog).

Why it exists: DVS tracks metadata history for auditability and rollback. This is
not part of daily usage but important for recovery.

---

### dvs merge-repo

Purpose: Import metadata + objects from another DVS repository.

Why it exists: Repo consolidation and migrations.

---

### dvs install / dvs uninstall / dvs git-status

Purpose: Shell completions and git integration helpers.

Why it exists: Convenience only.

## Deprecated commands (target)

The sync-first design removes the need for:
- dvs get
- dvs pull
- dvs push

These should not appear in daily usage or docs. `sync` is the single entry point.
