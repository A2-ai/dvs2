# dvs2-local log import — Spec v1

Source: [dvs2-local log migration — Design Model](https://claude.ai/artifact/WmiBRPQpCZpmLUt11G6ckL) (settled 2026-11-10).

This file is the implementation authority. Implementers follow it and do not redesign. The doc was the working copy; from here on this file wins. It builds on `design/migration.md` (R1–R35), which stays authoritative for everything this file does not change. Later decisions are appended as new sections that say what they supersede.

A `dvs` command that moves a dvs2-local project onto dvs-server by replaying its store's audit log, with recorded users and times, no migrate key and no commit walk.

## B — Problem

### B1 — dvs2-local projects take the dvs1 road

Today a dvs2-local project reaches the server only through `dvs-migrate`: an admin issues a migrate key, clones the repo and walks every commit on every ref. That machinery exists for dvs1, whose stores have no record of who added what.

### B2 — A dvs2 store already records its history

Every `dvs add` into a local store appends an entry to `<storage>/audit.log.jsonl`: path, blake3, user, time, message, compression, grouped by operation. `dvs init` appends an `init` entry naming the project. Rebuilding that history from commits is a worse copy of what the store already says.

### B3 — The log is not the whole story everywhere

- A store converted from dvs1 or devious with `dvs migrate local` is reused in place and writes no `init`: its log starts at the conversion and holds none of the older versions.
- Add entries name no project. If two projects were initialised on one store, its log mixes them.
- The log holds every add, including versions no commit ever referenced (orphans).

## D — Decisions

### D1 — It is a `dvs` command, not `dvs-migrate`

It runs in a checkout of the project, reads that project's store and log, and needs no clone and no commit walk.

*Decided — Wes, 2026-11-10.*

### D2 — History comes from the log, not from commits

Each `add` entry becomes one migrated version: its path, hash, user, time, message. The log's own order is kept, and one log operation becomes one server operation.

*Decided — Wes, 2026-11-10.*

### D3 — Only stores the log fully describes

It runs only when the log starts with an `init` entry and every blob in the store is named by an `add`. Otherwise it refuses and points at `dvs-migrate`: the store holds content from before dvs2 (B3). The blob check lists names and sizes only, as `orphans` does.

*Decided — Wes, 2026-11-10.*

### D4 — One project per store, or refuse

If the log's `init` entries name more than one project path, it refuses and names them; `--project-path <path>` says which one this checkout is. Paths are safe on a shared cluster: an `add` records the file's path relative to the project, and only `init` records the project's absolute path, once, when the project is created. Teammates' clones at other paths write no `init`, so one project normally has one.

*Decided — Wes, 2026-11-10.*

### D5 — Every logged add comes along

Every `add` in the log is migrated, whether or not a commit references it. Purging orphans is a dvs1 concern, where nothing recorded what was added; in dvs2 everything is tracked, so nothing is purged.

*Decided — Wes, 2026-11-10.*

### D6 — Server admins only

It runs with the user's normal dvs login, and the server allows it only for a server admin. A migration names versions after users who did not upload them to the server, so it needs more than write access. No migrate key.

*Decided — Wes, 2026-11-10.*

### D7 — Same server records as an admin migration

It uses the migration endpoints (runs, manifest, blobs, report), so its versions get the "migrated" tag, a run on the Migration page, and the run event in activity. The run's source reads "audit log".

*Decided — my call, reversible.*

### D8 — No commit, so no commit author

These versions carry no introducing commit. The "commit author" hover and the "not the commit author" attribution check do not apply to them; "not a directory user" still does.

*Decided — my call, reversible.*

### D9 — Runs behave as R35

One invocation is one run; runs never block; a run that dies shows as interrupted and running again does the rest; every call is safe to repeat.

*Decided — my call, reversible.*

### D10 — One command ports and switches

After the history is on the server, the checkout is pointed at the server project as `dvs migrate server` does today: backend swapped, compression rewritten, nothing committed. The flag that turns on the port is C1.

*Decided — Wes, 2026-11-10.*

## I — Invariants

### I1 — Every logged add is on the server or reported

After a run that ends cleanly, each `add` in the log is a migrated version on the server, or listed as missing with its reason (blob not in the store, hash mismatch).

### I2 — Local storage and the log are only read

The store, its blobs and `audit.log.jsonl` are never written or moved. Only the checkout's `dvs.toml` and sidecars change (D10).

### I3 — Repeating changes nothing

Running it again on the same log migrates nothing new and adds no event, as I6 in the migration spec.

### I4 — Who and when are the log's

A migrated version's recorded user and time are the log entry's, never the person running the command or the time it ran. The run records who ran it.

### I5 — A store it refuses is left untouched

Every refusal (D3, D4, D6, A3) happens before anything is sent to the server or written in the checkout.

## W — Walks

### W1 — An admin moves a dvs2-local project onto the server

1. The project exists on the server with its groups; the admin is signed in with `dvs login`.
2. In a checkout, on the default branch: `dvs migrate server proj https://dvs.example --import-history`.
3. It reads `dvs.toml`, opens the store's log, and checks D3 and D4. On a refusal it stops here and says why.
4. It starts a run, sends the log's adds in order as the manifest, and uploads each blob the server lacks.
5. It sends the report, which closes the run. The Migration page shows the run, source "audit log".
6. It switches the checkout to the server (D10) and prints a summary and the Migration page link.
7. The admin reviews and commits the `dvs.toml` and sidecar changes; teammates merge.

### W2 — It dies part way

1. The network drops after 300 of 1,000 versions.
2. The run shows as interrupted on the Migration page; the checkout is not switched.
3. Running the same command again is a new run: it sends the whole log, the server already has 300, the rest are uploaded, and the checkout is switched.

### W3 — A store converted from dvs1

1. The checkout was converted with `dvs migrate local`; its store has dvs1 blobs and a log that starts later, with no `init`.
2. `--import-history` refuses, names what it found (no `init`; N blobs in the store that no `add` names) and says to use `dvs-migrate`.

### W4 — A teammate after the switch

A teammate merges the switch commit; their checkout now points at the server and needs no command (`dvs migrate server` without `--import-history` is still there for a checkout that missed the merge).

## A — API

### A1 — The migration endpoints also accept a signed-in admin

`lookup`, `runs`, `manifest`, `blobs` and `report` accept either a migrate key (as now) or the user's normal dvs token when that user is a server admin. Any other token is 403 `admins only`.

### A2 — A manifest entry may have no commit

An entry from the log has no introducing commit; its recorded user and time come from the log (`recorded_*_source: audit_log`). The server stores it without commit fields and shows no commit author for it.

### A3 — A project is migrated one way

Start run carries the walk kind `audit_log` (beside `full` and `partial`), shown in the run's row on the Migration page. The two methods exclude each other:

- A project with commit-walk runs (`dvs-migrate`) refuses a log import. No override.
- A project with a log import refuses `dvs-migrate run`, naming who imported it and when, unless `--other-source` is passed (R17's flag, now also covering "migrated another way").

R17's different-source check does not apply to an `audit_log` run (there are no commits to compare).

*Decided — Wes, 2026-11-10 (reusing `--other-source`: my call, reversible).*

### A4 — Who ran it is recorded

The run's "started by" is the signed-in admin, as with a key it is the key's issuer.

## C — CLI

### C1 — `dvs migrate server <name> <url> --import-history`

The flag imports the store's audit log to the server project, then the checkout is switched (D10). Without it the command is unchanged. The help text says the history comes from the store's audit log.

*Decided — Wes, 2026-11-10.*

### C2 — Output

The same shape as `dvs-migrate run`'s summary: versions in the log, already on the server, uploaded, missing (with reasons), then the switch, then the Migration page link. `--json` gives the report document.

*Decided — my call, reversible.*

### C3 — Refusals say what to do

- Not a server admin: says so, and that an admin must run it.
- Store not fully described by its log (D3): names what was found and says to use `dvs-migrate`.
- More than one project in the log (D4): names the paths.
- The project was migrated with `dvs-migrate` (A3): says so.
- Not a dvs2-local checkout (dvs1, devious, already on a server): the existing `dvs migrate` refusals.

All exit 2 and change nothing.

*Decided — my call, reversible.*

### C4 — Retries and timeouts as R18

The same retry and timeout behaviour as `dvs-migrate`, shared rather than re-written.

*Decided — my call, reversible.*

## Q — Settled design-wide questions

### Q1 — Converted dvs1 stores stay with `dvs-migrate`, unchanged

Stores that fail D3 use `dvs-migrate` as it is: it already prefers the log's user and time for any version the log names.

*Decided — Wes, 2026-11-10.*

## Appendix — Implementation phases (`P`)

| Phase | What | Depends on | Files |
|---|---|---|---|
| P1 | Wire: `ManifestEntry.commit` optional; `WalkKind::AuditLog`; the shared retrying HTTP helper (C4) moved into the `dvs` crate | — | `dvs/src/migration.rs`, a new `dvs` module for the retrying client, `dvs/src/lib.rs` |
| P2 | Server: A1 (admin token on migration endpoints), A2 (commit-less entries stored and shown), A3 (one method per project; `audit_log` runs skip R17), page shows the source | P1 | `dvs-server/src/**`, `templates/**`, `migrations/0002_migration.sql` |
| P3 | `dvs migrate server --import-history`: read and check the log (D3, D4), import (D2, D5, D9), switch (D10), output and refusals (C1–C3) | P1 | `dvs/src/migrate.rs` (or a new `dvs/src/migrate/` submodule), `dvs-cli/src/main.rs` |
| P4 | `dvs-migrate`: use the shared retrying client; A3's refusal with `--other-source` | P1 | `dvs-migrate/src/**` |

P2, P3 and P4 are disjoint and run in parallel after P1.

## Resolutions from implementation (L1–L3)

### L1 — A manifest entry names its log operation

D2's "one log operation becomes one server operation" needs the operation on the wire: `ManifestEntry` gains `operation` (the log entry's `operation_id`, omitted when absent). The server groups commit-less entries by it the way it groups commit entries by commit.

*Decided — orchestrator, follows D2.*

### L2 — `--project-path` asserts the log is one project

With several `init` paths (D4), `--project-path <path>` must name one of them, and then every `add` in the log is imported as this project's: the case it exists for is one project re-initialised at a new path. Any other value is refused.

*Decided — orchestrator, small call, reversible.*

### L3 — A logged version's size comes from its blob

The log records no size. A version's size is the length of its content, decompressed when the blob is zstd, as `dvs-migrate` sends dvs2-local content (A7).

*Decided — orchestrator, follows A7.*

## Resolutions from the P2 review (L4–L6)

### L4 — A report keeps its run's method

A run's report may change its walk between `full` and `partial` only. A report naming `audit_log` for a commit-walk run, or `full`/`partial` for an `audit_log` run, is 400 `invalid report: a run's walk cannot change between a commit walk and an audit-log import`.

*Decided — orchestrator, follows A3.*

### L5 — A3 counts runs that migrated something

"A project with commit-walk runs" (A3) means runs that migrated at least one version, the same runs R16's history counts. A failed or empty `dvs-migrate` run does not block an import, and the reverse. `lookup`'s `previous` gains `imported_from_log`; when it is true, `dvs-migrate` skips R17's local different-source check and the server's A3 answer decides.

*Decided — orchestrator, small call, reversible.*

### L6 — Commit ids are hex

A manifest entry's `commit.id` must be 4–64 lowercase hex characters (400 otherwise), so a commit group can never collide with a `log:` operation group. The Migration page's Scope panel shows "audit log" for an import's walk and leaves out "Refs walked".

*Decided — orchestrator, small call, reversible.*

## Resolutions from the P3 review (L7–L8)

### L7 — One migrated version per path and hash

A log that adds the same (path, hash) more than once imports it once, from its earliest `add` (its user, time, message and operation), as migration D6 does for a commit walk. Later repeats are counted in the summary as repeats, not sent.

*Decided — orchestrator, follows migration D6.*

### L8 — The server is asked before the store is read

`--import-history` calls `lookup` and starts the run before it reads any blob, so a non-admin (D6) or a project migrated the other way (A3) is refused without waiting for the store to be read. A log entry whose path dvs cannot accept is a refusal (exit 2) before any call.

*Decided — orchestrator, small call.*

## Recorded at the end of implementation (L9)

### L9 — Calls made while building, recorded

- A run with missing versions still reports, switches the checkout and exits 0 with outcome `partial`, as `dvs-migrate run` does; missing content is listed, not fatal (migration D8).
- The run's `scope.source` is the audit log's path; the Migration page names the source "audit log" from the run's walk.
- Uncompressed blobs are hash-checked by the server on upload (I3 of the migration spec), not read twice locally; zstd blobs are checked after decoding.
- `--import-history` cannot be combined with `--upload-missing` or `--storage`; `--project-path` requires `--import-history`.
- A store with no audit log, or a log that does not parse, is refused like D3.
- `--json` prints the report document with a `switch` key for the checkout's switch.
- A server refusal that arrives after the run has opened exits 1 as an interruption (W2), not 2.
- `0002_migration.sql` is edited in place: it is unreleased.

*Decided — orchestrator, small calls, recorded after the divergence inventory.*

## Resolutions after the evidence run (L10–L11)

### L10 — An import keeps every add, reverts included (supersedes L7)

On the server, ordinary adds keep one upload per operation, so a file set back to an earlier version shows in its history. Migrated versions from a commit walk stay one per (path, hash) (migration D6). An audit-log import instead migrates **every** `add`: its versions are identified by (path, hash, log operation). The log's operation ids are stable, so running the import again still changes nothing (I3).

- `EntryRef` gains `operation`, set for import entries; `lookup` answers `migrated` per (path, hash, operation) for them.
- The server matches what an ordinary revert would leave: one upload per add, in its own operation. The change is kept small: no new upload column; the one-migrated-version rule stays for commit-walk versions, and import versions (no commit) are unique per (project, path, hash, recorded time), which keeps a re-run idempotent. The audit log records time to the second, so a file set back to content it had within the same second (A, B, A in one second: only a script does this) imports as one version; this is accepted (Wes, 2026-11-10).
- The summary's "Repeated adds" line (L7) is removed.

*Decided — Wes, 2026-11-10.*

### L11 — Running the import again in a switched checkout changes nothing

When the checkout already points at the same server and project and has no local store, `--import-history` says `Already migrated: nothing changed` and exits 0, as plain `dvs migrate server` does. A checkout pointing at a different server keeps the existing refusal.

*Decided — Wes, 2026-11-10.*
- With a same-second collapse, the client counts the collapsed add as not yet migrated (lookup answers per log operation), so a re-run's totals and an interrupted run's "N of M" can be one high per collapse. Nothing is stored twice. Accepted with the collapse.
