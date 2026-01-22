# DVS Commands: What They Do and When You Need Them

This guide explains the DVS commands, why they exist, and which ones are
intended for **daily use** versus **rare/advanced** cases. The goal is a
sync‑first workflow where most people only need a small set of commands.

## Daily usage (the normal workflow)

### `dvs init`

**Purpose:** One‑time setup. Creates the repo config and wires storage.

**Why it exists:** DVS separates storage from tracking, so the repo needs to know
where the external storage lives. This is only needed when starting a repo or
changing storage settings.

**Typical usage:** Run once per repo.

---

### `dvs add <files...>`

**Purpose:** Publish a new data version into DVS.

**What it does:**

- Hashes data files (content‑addressable storage).
- Copies data into the external storage directory.
- Writes metadata (`.dvs` / `.dvs.toml`) for Git to track.
- Updates `.gitignore` so data files are not committed.

**Why it exists:** DVS keeps Git small by tracking metadata only. `add` is the
explicit “publish” step that records a data version and makes it syncable.

**Typical usage:** Run when you intentionally want to version a file.

---

### `dvs sync`

**Purpose:** Make local + remote converge (push by default).

**Default flow:** pull missing objects → materialize → push new objects

**Why it exists:** This replaces `push`/`pull`/`get` with a single operation that
matches DVS’s “latest wins” data workflow.

**Typical usage:** Run after `add`, or when you want the latest data locally.

---

### `dvs status`

**Purpose:** Show discrepancies between local, metadata, and (optionally) remote.

**Why it exists:** DVS separates storage from tracking, so you need a fast way
to see what is missing, dirty, or outdated before syncing.

**Target behavior (sync‑first):**

- Local vs metadata (dirty/absent/current)
- Cache/remote availability where relevant

**Typical usage:** Run before sync or when debugging mismatches.

## Rare / advanced usage (not daily)

### `dvs materialize`

**What it is:** Copies cached objects from `.dvs/cache` into the working tree.

**Why it exists:** DVS can store objects locally without overwriting files
immediately. `materialize` lets you explicitly restore files from the cache:

- **Partial restores** (only some files)
- **No overwrite** unless you choose to materialize
- **Offline usage** (objects already cached)

**Daily usage?** Usually no. In a sync‑first CLI, `materialize` is mostly an
internal step or advanced tool.

---

### `dvs log` / `dvs rollback`

**Purpose:** Audit and restore DVS metadata history.

**Why they exist:** DVS tracks history of metadata changes (not raw file merges).
These commands are useful for audits, debugging, or reverting a bad data update.

**Daily usage?** No.

---

### `dvs merge-repo`

**Purpose:** Import tracked files, metadata, and objects from another DVS repo.

**Why it exists:** Repo consolidation and migrations. Not day‑to‑day.

**Daily usage?** No.

---

### `dvs install` / `dvs uninstall`

**Purpose:** Install shell completions and git status shim.

**Daily usage?** No (one‑time).

## Command minimalism summary

For most users, the **daily workflow** should be:

1. `dvs add` to publish new data versions
2. `dvs sync` to converge local+remote
3. `dvs status` to see discrepancies

Everything else exists to support setup, auditing, or special workflows.

## Notes on design docs

The `design/` folder contains historical R‑first docs that describe older
workflows (`dvs_get`, YAML config, per‑directory `.gitignore` handling). Those
docs are useful context, but they are not fully aligned with the current
sync‑first direction. The CLI plan (`plans/048-cli-sync.md`) captures the new
shape.
