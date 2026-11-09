# dvs Migration — Spec v1

Source: [dvs Migration — Design Model](https://claude.ai/artifact/FowXzLwwjynfeC3XYBuqGw) (settled 2026-11-05).

This file is the implementation authority. Implementers follow it and do not redesign. The doc was the working copy; from here on this file wins. Later decisions are appended as new sections that say what they supersede; earlier text is not edited in place.

How a project on local storage (dvs1 or dvs2) moves to dvs-server: an admin-run migration that keeps who-added-what, and a one-time `dvs migrate` that switches users over.

**Personas.** An IT admin clones projects and runs `dvs-migrate`. The project lead runs `dvs migrate` on the default branch; teammates merge. (Q1)

## B — Problem

### B1 — Local storage gives no server benefits

A project on local storage (a `storage_dir` on a shared filesystem) has no permissions model, no audit log and no web UI. Moving it to dvs-server today means re-adding every file, which loses who added it and when.

### B2 — History is spread across every ref, not the tip

A sidecar's content is needed at every commit where that sidecar exists. Migrating only branch tips leaves older commits pointing at content the server never received.

### B3 — History mixes three sidecar shapes

One repo can hold sidecars written by dvs1 v0.0.1 (`<file>.dvs` beside the file: `blake3_checksum`, `file_size_bytes`, `time_stamp`, `message`, `saved_by`), dvs1 v0.0.2+ (same place: `size`, `add_time` instead), and dvs2 (`.dvs/` folder: `hashes.blake3`, `size`, `created_by`, `add_time`, `compression`, `message?`). Config is `dvs.yaml` (dvs1) or `dvs.toml` (dvs2).

### B4 — dvs1's recorded user is the file's OS owner

dvs1 `saved_by` is the owner of the data file at add time, not necessarily who ran `add`. Provenance from the sidecar alone can be wrong on shared directories.

### B5 — Users have no path from dvs1 to a server

dvs2 does not read `dvs.yaml` or dvs1 sidecars. A dvs1 project, once its content is on the server, has no way for users to reach it.

## D — Decisions

### D1 — Admin migration is a standalone binary

A separate binary (working name `dvs-migrate`) does the history walk, report and upload. It talks to dvs-server over HTTP; it is not a `dvs` subcommand and not a server-side job.

*Decided — Wes, 2026-11-05.*

### D2 — Migrate key: one project, reusable, 8h after first use

The key only identifies one project and only opens the migration endpoints. It works for any number of runs (dry-run, resume, catch-up) and expires 8 hours after its first use. Issuing a new key revokes the old one.

*Decided — Wes, 2026-11-05 (revoke-on-reissue: my call, reversible).*

### D3 — The walk covers every commit on every ref

Every commit reachable from the walked refs is visited, not just tips, so every hash that ever appeared in a sidecar is found. Refs walked: `refs/heads` and `refs/remotes`, plus `refs/tags` (cheap to include, rarely adds anything). Never stashes. The report lists every ref walked.

*Decided — Wes, 2026-11-05 (including tags: my call, reversible).*

### D4 — Sidecars are recognized by content, not by name

A `.dvs` file is classified by its JSON fields as dvs1 v0.0.1, dvs1 v0.0.2+ or dvs2. Anything else is reported as unrecognized, not guessed at.

*Decided — my call, reversible.*

### D5 — Provenance keeps both the sidecar and the commit

For each add, the sidecar's user and time (`saved_by`/`created_by`, `add_time`/`time_stamp`) and the author and date of the commit that introduced that sidecar version are both kept. The sidecar's values are primary; the commit's fill in when the sidecar has none. For dvs2-local projects, the store's `audit.log.jsonl` (operation, user, time per add) is read too and wins over the sidecar when it has the same path and hash.

They are expected to agree; the report and the UI flag every entry where the sidecar user differs from the commit author, with a count in the totals. How often they diverge will be checked on a real dvs1 project during implementation.

*Decided — Wes, 2026-11-05.*

### D6 — One imported add per path and version

The server's audit log gets one "added (imported)" entry for each (path, hash) at the commit where it first appears, with its original user and time. Run-level events are defined in A6.

*Decided — my call, forced by the server: `get` only serves a (path, hash) that has an upload (A3).*

### D7 — Content is uploaded under its recorded hash

All three shapes use blake3, so content goes up under the hash already in the sidecar. Nothing is re-hashed locally; the server checks the hash on receipt.

*Decided — my call, reversible.*

### D8 — Missing content is reported, not fatal

A hash found in history whose file is not in local storage (or fails its hash check) is listed in the report as missing. The rest still uploads. A migration can be finalized with hashes missing once the admin accepts the list. The accepted list, and each run's problems (missing, corrupt, unrecognized), are recorded on the server as part of the audit trail.

*Decided — Wes, 2026-11-05.*

### D9 — Report data lives on the server as records

The binary writes the report locally (JSON plus a readable summary) for the admin. Everything the report holds (scope, totals, entries, problems) is also stored on the server as records tied to the run, so the audit trail can be searched there. It is not an uploaded copy of the file.

*Decided — Wes, 2026-11-05.*

### D10 — Runs are resumable and idempotent

Re-running uploads only what the server lacks and writes only audit entries it does not have. A run interrupted midway is finished by running again.

*Decided — my call, reversible.*

### D11 — Finishing: catch-up run, then local storage goes read-only

The last run before cutover must find nothing new. Only then is the migration marked complete, and local storage made read-only so no dvs1 or dvs2-local add can land there afterwards. The binary never changes permissions. Finalize refuses while storage is writable and prints the recommended command; making it read-only is the admin's step.

*Decided — Wes, 2026-11-05.*

### D12 — Local storage stays after migration, read-only

It is not deleted. Old checkouts keep working through their lockfile's dvs1 package.

*Decided — Wes, 2026-11-05.*

### D13 — dvs2 never reads dvs1 in normal operation

`status`, `add`, `get` and the rest know only `dvs.toml` and `.dvs/`. dvs1 knowledge lives only in the migration binary and `dvs migrate`.

*Decided — Wes, 2026-11-05.*

### D14 — `dvs migrate` converts the working tree and never commits

It needs no existing dvs2 config. It writes `dvs.toml` and moves dvs1 sidecars into `.dvs/`. Committing, stashing or discarding is the user's git step.

*Decided — Wes, 2026-11-05.*

### D15 — One `dvs migrate`, destination as a subcommand

The destination mirrors `dvs init`; the source (dvs1 or dvs2-local) is detected from the tree.

- `dvs migrate server <name> <url>`: from dvs1 or dvs2-local.
- `dvs migrate local [path]`: from dvs1 only. The path defaults to `storage_dir` in `dvs.yaml`, and `permissions`/`group` carry over. A dvs1 store is already a valid dvs2 local store (same `<hash[..2]>/<hash[2..]>` layout, uncompressed blobs, dvs2 local detects per-blob compression), so no data moves.

Combinations with nothing to do (dvs2-local → local, anything already on a server) are refused with the reason. `dvs attach` is unchanged.

From dvs2-local, `server` swaps the backend in `dvs.toml` and rewrites each sidecar's `compression` to the server project's setting: dvs2 decompresses a download using the sidecar's `compression`, and a local store may hold uncompressed blobs, so the old value would be wrong.

*Decided — Wes, 2026-11-05.*

### D16 — dvs1 config without dvs2 config is an error

When `dvs.yaml` exists and `dvs.toml` does not, every dvs2 command stops with an error that names `dvs migrate`.

*Decided — Wes, 2026-11-05.*

### D17 — `dvs migrate` at any commit is the recovery path

If local storage is lost, checking out an old commit and running `dvs migrate server` makes it fetchable from the server.

*Decided — Wes, 2026-11-05.*

### D18 — `dvs migrate server` warns about hashes the server lacks

After converting, it checks every sidecar's hash against the server and lists any missing; the conversion stands.

When a missing hash is still in local storage, `--upload-missing` uploads it as an ordinary add by the user. Its message is filled with a standard note: not found in the admin's migration, found in local storage by the user, plus the sidecar's original user and time. The audit trail then reads "missed by the admin's pass, found and uploaded by <user>", with no server change. Without the flag, it only lists them.

*Decided — Wes, 2026-11-05 (flag name and note wording: my call, reversible).*

### D19 — dvs1 `.gitignore` entries are kept

dvs1 wrote each data file into a `.gitignore`. `dvs migrate` leaves those entries; the data files must stay ignored.

*Decided — my call, reversible.*

### D20 — A file with both a dvs1 and a dvs2 sidecar

On a half-merged branch a file can have both. If their hashes match, the dvs1 one is dropped; if they differ, `dvs migrate` leaves that file alone and reports it.

*Decided — my call, reversible.*

### D21 — One dvs1 parser, shared

A new `dvs-migrate` crate in the dvs2 workspace, with the dvs1 sidecar/config parser in the `dvs` library, used by both the binary and `dvs migrate`. The binary reuses dvs2's server client.

*Decided — Wes, 2026-11-05.*

### D22 — `dvs migrate` is CLI only for v1

The dvs2 R package has no login, `attach` or server init. Revisit if it gets server init.

*Decided — Wes, 2026-11-05.*

### D23 — dvs1's unreleased audit-log branch is ignored

The unmerged `audit-log` branch writes `storage_dir/audit.log`. The prism `dvs_0.0.2.9000` tarball (the build most people use) has Rust source identical to the clone's main and no audit log.

*Decided — Wes, 2026-11-05.*

## I — Invariants

### I1 — The migration never writes to the repo or local storage

The binary only reads git and `storage_dir`. It works on its own mirror clone in a temp dir (Q2), never fetching into or changing the admin's clone. Everything it writes goes to the server, the temp dir or the report file.

### I2 — Every hash in history is uploaded or listed missing

After a run, each hash found in any sidecar on any walked commit is either on the server or in the report's missing list. Nothing is silently skipped.

### I3 — The server stores only content whose blake3 matches

Uploaded bytes that do not hash to the claimed blake3 are rejected and reported as corrupt.

### I4 — Imported adds never look like live adds

Every audit entry the migration writes is marked as imported and names the migration run. The original user and time are shown as recorded, never as the admin and now.

### I5 — A migrate key reaches one project and only migration endpoints

It cannot read or write any other project, and cannot call normal `add`/`get` endpoints.

### I6 — Re-running never duplicates

The same history, walked twice, produces the same server state and no extra audit entries beyond the run's own event (and none at all if nothing changed).

### I7 — `dvs migrate` never loses a sidecar

Every dvs1 sidecar in the working tree is either converted into `.dvs/` or left in place and reported (D20). It never deletes one it did not convert.

### I8 — dvs2 behavior outside `dvs migrate` is unchanged

Apart from the D16 error, no existing dvs2 command changes what it reads, writes or prints.

## F — Fold: history to report

### F1 — Refs become one set of commits

The walked refs (D3) are expanded to every reachable commit, each visited once however many refs reach it.

### F2 — Each commit's sidecars are found from its own config

dvs1 sidecars are `*.dvs` files anywhere in the tree. dvs2 sidecars are under that commit's metadata folder (its `dvs.toml` `metadata_folder_name`, default `.dvs`). A sidecar git blob already parsed is not parsed again.

*Decided — my call, reversible.*

### F3 — Entries are keyed by (path, hash)

Each distinct (data-file path, blake3) becomes one entry. Its introducing commit is a commit where the pair is present and absent from every parent; with several, the oldest author date wins. The entry carries the sidecar's user, time, message and size, plus that commit's author, date and id.

*Decided — my call, reversible.*

### F4 — Storage location comes from the config in history

Each entry's content is looked up in the `storage_dir` (dvs1) or local backend path (dvs2) named by the config at its introducing commit, using that shape's layout. The report lists every storage path seen in history; the admin remaps any of them with `--storage-map old=new`. If flags pile up, the same settings can come from a TOML file.

*Decided — Wes, 2026-11-05 (TOML file: my call, reversible).*

### F5 — The report summarizes, then lists

| Part | Contents |
| --- | --- |
| Scope | refs walked, commit count, storage paths seen and their remaps, whether the walk was full or partial (Q2) |
| Totals | sidecars by shape, unique hashes, total bytes, already on server, to upload, sidecar-user/commit-author mismatches (D5) |
| Entries | one row per (path, hash) with provenance (F3) |
| Problems | missing content, corrupt content (hash mismatch), unrecognized `.dvs` files, size mismatches, unmatched user names (A4) |

A dry run produces the same report without uploading.

## W — Walks

### W1 — Admin migrates a project

1. Admin creates the project on dvs-server and sets it up (members, groups).
2. On the project's Migration page, admin issues a migrate key (D2, U1).
3. Admin runs `dvs-migrate plan` against a repo (path or URL, Q2) and the storage path(s), with the key. The binary walks history (F1–F4) and writes the report (F5).
4. Admin reads the report and fixes what they can (remap storage paths, fix unreachable remotes).
5. Admin runs `dvs-migrate run`. The binary sends content the server lacks, the imported adds, and the report data. The server records a run event (A6).
6. Repeat 3–5 as needed; each run only sends what is new (D10).
7. Admin makes local storage read-only, then runs `dvs-migrate finalize` (nothing new, accepts any missing hashes) (D8, D11). The project shows as migrated.
8. Admin hands off to the project lead, who runs `dvs migrate server` on the default branch (W2).

### W2 — User moves a dvs1 project onto the server

1. User has dvs2 installed and is signed in to the server.
2. On a branch, user runs `dvs migrate server <name> <url>`.
3. It writes `dvs.toml`, moves the working tree's dvs1 sidecars into `.dvs/`, removes `dvs.yaml` and the converted dvs1 sidecars (unless `--no-clean`), and lists any hashes the server lacks (D18).
4. User reviews and commits the change with git.
5. Other branches get the change by merging, or by running `dvs migrate server` on them.

### W3 — User attaches a dvs2-local project to the server

Same as W2, but step 3 swaps the backend in `dvs.toml` and rewrites each sidecar's `compression` (D15); no sidecars move.

### W4 — Reading an old commit after local storage is gone

1. User checks out the old commit.
2. User runs `dvs migrate server <name> <url>` (D17).
3. `dvs get` fetches from the server.
4. User discards the working-tree change when done; nothing is committed.

### W5 — Someone runs dvs2 on an unmigrated dvs1 checkout

Any dvs2 command stops with an error naming `dvs migrate` (D16).

### W6 — A dvs1 project moves to dvs2 local storage

`dvs migrate local` writes a local-backend `dvs.toml` pointing at the existing `storage_dir` and converts the sidecars (D15). No data moves and no server is involved.

## A — API (dvs-server)

Today every audit row's actor is the caller and every time is `now()`; tokens have no scope; `get` serves a blob only for a (path, hash) with a completed upload in the project. The items below are what the migration adds.

### A1 — Migrate keys are a new, scoped token kind

Only an admin (member of `groups.admin_group`) can issue one, from the project page. Stored hashed like API tokens, bound to the project and the issuing admin. Expires 8 hours after first use, or 7 days after issue if never used. Revoked by reissue or by finalize.

*Decided — Wes, 2026-11-05 (7-day unused expiry and revoke-on-finalize: my call, reversible).*

### A2 — Migration endpoints accept only a migrate key

Under `/api/{name}/migration/`, separate from the normal API; a normal token is refused there and a migrate key is refused everywhere else (I5).

| Call | Sends | Gets back |
| --- | --- | --- |
| start run | binary version, plan totals | run id |
| manifest | run id, entries (path, size, hash, provenance from F3) | hashes the server lacks; entries already present are skipped |
| blob upload | run id, hash, uncompressed bytes | created / already had; hash mismatch is refused |
| report | run id, the F5 report data | stored as records (D9) |
| finalize | run id, accepted missing hashes | migrated, or why not |

*Decided — my call, reversible.*

### A3 — Imported adds are real file versions

Each imported (path, hash) becomes a completed upload in the project, so `get`, file versions and `history` work for it with no special case. That is also why D6's unit must be (path, hash): `get` refuses a pair with no upload row. The row is marked imported, linked to its run, and carries the recorded user, recorded time, introducing commit, commit author and commit date.

*Decided — my call, reversible (forced by the server's download rule).*

### A4 — Recorded users are matched to directory users by name

dvs1/dvs2 sidecars record an OS username. Where it exactly equals a directory user's username, the import links to that user; otherwise it keeps the name as text, shown with an "unmatched" tag. The report lists every unmatched name.

*Decided — Wes, 2026-11-05.*

### A5 — Imported adds are grouped by introducing commit

Within a run, the imports for one introducing commit form one operation, with that commit's id as its git commit. Each file's own sidecar message stays on its import record.

*Decided — my call, reversible.*

### A6 — The migration itself is in the project's event log

Each run that changes anything adds a project event "migration run by <admin>" with its totals; finalize adds "project migrated by <admin>". These sit with created / access / lock events.

*Decided — my call, reversible.*

### A7 — The server compresses, as it does for any upload

The binary sends uncompressed bytes: dvs1 content as stored, dvs2-local content decompressed first when its sidecar says `zstd`. The server hashes, then compresses per the project's setting.

*Decided — my call, reversible.*

### A8 — A locked project refuses migration uploads

Same rule as normal uploads. Normal adds by users are not blocked during a migration; the server dedupes either way.

*Decided — my call, reversible.*

### A9 — `history` returns the recorded user and time

Recorded user and time as-is, no new field; only the server UI shows the imported tag.

*Decided — Wes, 2026-11-05.*

## C — CLI

### C1 — `dvs-migrate plan` walks and reports, sends nothing

Inputs: repo (path or URL, Q2), migrate key, storage remaps (F4). Writes the report (F5). Reads the server only to learn which hashes it already has.

### C2 — `dvs-migrate run` uploads what the plan found

Same inputs. Walks again (so a stale plan can't be uploaded), uploads missing content and imported adds, sends the report data. Prints the totals and the report path.

### C3 — `dvs-migrate finalize` closes the migration

Runs `plan`; refuses if anything is left to upload, if the last walk was partial (Q2), or if storage is still writable (D11). Asks the admin to accept the missing list, if any (D8), then marks the project migrated.

### C4 — Machine-readable output

All three take `--json` and print the report as JSON, matching dvs2's existing `--json` conventions.

*Decided — my call, reversible.*

### C5 — `dvs migrate` in dvs2

`dvs migrate server <name> <url>` and `dvs migrate local [path]` (D15). They convert the working tree (D14, D19–D20), and `server` then checks every hash on the server (D18, `--upload-missing`). `server` signs in with the user's normal dvs2 login, not a migrate key. It cleans up: deletes `dvs.yaml` and every dvs1 sidecar it converted. `--no-clean` keeps both.

*Decided — Wes, 2026-11-05.*

### C6 — `dvs migrate` on an already-migrated tree

If `dvs.toml` already points at the same server and project and no dvs1 sidecars remain, it changes nothing and says so. If `dvs.toml` points elsewhere, it refuses unless told to replace the backend.

*Decided — my call, reversible.*

## U — UI (dvs-server)

### U1 — An admin-only Migration page per project

At `/project/{name}/migration`, beside the groups and lock pages. It shows: issue key (the key is shown once), key state (unused / active until <time> / expired / revoked), each run with its totals, the latest report's data with a download, and whether the project is migrated.

*Decided — my call, reversible.*

### U2 — Imported versions show who and when as recorded

In activity, the global audit log, the CSV export and file versions, an imported version shows the recorded user and time, an "imported" tag, and a link to its run. Hovering or expanding shows the commit author and date (D5).

*Decided — my call, reversible.*

### U3 — Migration events appear with other project events

The A6 events show in the project's activity and the global audit log like lock and access events.

*Decided — my call, reversible.*

### U4 — The UI does not decide completeness

It shows the binary's report and the finalize result. It never computes what is missing itself; finalize goes through the binary (C3) because only the binary can see the repo and local storage.

*Decided — my call, reversible.*

## Settled design-wide questions

### Q1 — Who lands the cutover commit

An IT admin clones projects and runs `dvs-migrate`; the project lead runs `dvs migrate server` on the default branch and teammates merge. This goes in the recommended instructions, not the tools.

*Decided — Wes, 2026-11-05.*

### Q2 — What the binary needs from the repo

Both a local path and a git URL are accepted, and both get everything: no "walk as-is" mode. The binary always works on its own mirror clone in a temp dir.

- Given a URL, it mirrors from that URL.
- Given a path, it mirrors from that clone's remotes and also takes the clone's own local branches, so unpushed work is included.

Only when a remote can't be reached or the source is shallow does it walk what it has; the report then says so up top, and finalize refuses until a full run has succeeded.

*Decided — Wes, 2026-11-05 (mirror-clone mechanism and finalize rule: my call, reversible).*

## Appendix — Implementation phases (`P`)

Dependency-ordered. Derived from the model at persist time; not part of the reviewed doc.

| Phase | Builds | Depends on | Touches |
| --- | --- | --- | --- |
| P1 | dvs1 parser: config (`dvs.yaml`), both dvs1 sidecar shapes, shape classifier (D4, D21) | — | `dvs2/dvs/src/legacy/` (new), `dvs2/dvs/src/lib.rs` |
| P2 | dvs-server: migrate keys, runs, import records, report records, new project event kinds; migration endpoints (A1–A8); `history` returns recorded user/time (A9) | — | `dvs-server/migrations/` (new file), `dvs-server/src/db/` (new migration module, audit/projects views), `dvs-server/src/auth/` (migrate key), `dvs-server/src/webserver/api.rs` + new migration router |
| P3 | dvs-server UI: Migration page, imported tags in activity / audit / CSV / file versions (U1–U4) | P2 | `dvs-server/templates/`, `dvs-server/src/webserver/web.rs` |
| P4 | `dvs migrate server|local`, D16 guard, `--no-clean`, `--upload-missing` (D14–D20, C5–C6) | P1 | `dvs2/dvs/src/migrate.rs` (new), `dvs2/dvs/src/config.rs` (guard), `dvs2/dvs-cli/src/main.rs` |
| P5 | `dvs-migrate` binary: mirror clone, walk, fold, report, plan / run / finalize (D1–D11, F1–F5, C1–C4, Q2) | P1, P2 (API contract) | `dvs2/dvs-migrate/` (new crate), `dvs2/Cargo.toml` (workspace member) |

P1 and P2 run in parallel. P3, P4 and P5 can run in parallel after their dependencies; P4 and P5 both register in `dvs2/Cargo.toml` only if P4 needs a new dependency, so coordinate that one file.

## Contract — migration API, report data and shared types (pinned before implementation)

Pins the wire shapes of A1–A8, C1–C4, D5, D8, D9, D11 and F3–F5 so P2, P4 and P5 can be built in parallel. It supersedes nothing above; where it fills a gap the earlier items left open, it says so. All JSON keys are snake_case. All times are RFC 3339 in UTC with a `Z` suffix (the form `jiff::Timestamp` prints, as `FileMetadata.add_time` already does); fractional seconds are kept as recorded. Hashes are blake3, 64 lowercase hex characters (`validate_blob_hash`). Paths are project paths (`dvs::ProjectPath`: repo-relative, `/`-separated).

### K1 — Error bodies keep dvs-server's shape

Every refusal below is dvs-server's existing `(StatusCode, String)`: a status and a plain-text body, not JSON, so the existing client helpers (`status_error`, `response_error`) read them unchanged. Where the binary must tell refusals with the same status apart, the body starts with a fixed prefix, as `unknown project` does today; the table gives the prefix, and the binary matches on it. Only the finalize result (K9) is JSON, because it is a result, not an error.

The binary handles 401, 403, 404, 409 and 423 from migration endpoints itself, before falling back to `status_error`: that helper maps 401 to "run `dvs login`" and an unknown 404 to "server too old", both wrong for a migrate key.

### K2 — Migrate key: format, header, and how the server tells it apart

| | |
| --- | --- |
| Format | `dvsm_` followed by 30 ASCII alphanumerics (the `generate_auth_token` alphabet and length), 35 characters in all |
| Header | `Authorization: Bearer <key>`, as API tokens |
| Telling kinds apart | By prefix. API tokens are alphanumeric only, so no API token can start with `dvsm_`. The server checks the prefix before any lookup. |
| Storage | blake3 of the whole key, as `tokens.token_hash` (A1); the key itself is shown once (U1) |
| First use | The first request that passes K3's checks, on any migration endpoint including `lookup`. It starts the 8-hour clock (D2, A1). |

### K3 — Key refusals

Checked in this order. "Migration endpoint" means any route under `/api/{name}/migration/`; "normal endpoint" is every other route that takes a bearer token.

| Case | Status | Body starts with |
| --- | --- | --- |
| No `Authorization: Bearer` header | 401 | `missing bearer token` (existing) |
| Project in the URL unknown | 404 | `unknown project` (existing) |
| An API token (no `dvsm_` prefix) on a migration endpoint | 403 | `not a migrate key` |
| A migrate key on a normal endpoint (I5) | 403 | `a migrate key only opens the migration endpoints` |
| `dvsm_` key not found | 401 | `invalid migrate key` |
| Revoked (by reissue or finalize) | 401 | `migrate key revoked` — then ` at <time>: reissued` or ` at <time>: project migrated` |
| Expired (8h after first use, or 7 days after issue unused) | 401 | `migrate key expired` — then ` at <time>` |
| Key bound to another project | 403 | `migrate key is for another project` (the other project is not named) |
| Issuing admin no longer active in the directory | 403 | `not an active user in the directory` (existing `User::from_lookup`) |
| Issuing admin no longer an admin | 403 | `admins only` (existing `require_admin`) |
| Project locked, on `manifest` and `blobs` only (A8) | 423 | the existing `locked_error` text: `Project locked by <user> on <date>[: <reason>]` |

A request made with a migrate key acts as the issuing admin wherever dvs-server needs an actor (operation `created_by`, project events).

### K4 — Endpoints

All under `/api/{name}/migration/`, all JSON in and out except the blob body. They mirror the normal API's shapes: run id travels where `operation_id` does today (JSON body for manifest, query string for blobs).

| Call | Method and path | Request | Success | Writes? |
| --- | --- | --- | --- | --- |
| lookup | `POST lookup` | K5 | 200 | No (except first use, K2) |
| start run | `POST runs` | K6 | 201 | Yes |
| manifest | `POST manifest` | K7 | 200 | Yes |
| blob upload | `PUT blobs/{hash}?run_id=<uuid>` | uncompressed bytes, `Content-Type: application/octet-stream` | 201 created / 200 already had | Yes |
| report | `POST report` | K8 | 200 | Yes |
| finalize | `POST finalize` | K9 | 200 | Yes |

Statuses common to every call besides K3: 400 for a malformed body, bad hash or bad path (existing texts, e.g. `invalid blob hash: "…"`); 413 when the JSON body is over 64 MiB (`DefaultBodyLimit`, as `manifest` today); 500 `internal server error` (existing `internal_error`).

Statuses for calls that carry a `run_id`:

| Case | Status | Body starts with |
| --- | --- | --- |
| No run with this id in this project | 404 | `unknown migration run` |
| Run is not open (reported or interrupted, K6) | 409 | `migration run <id> is not open` |

### K5 — `lookup`: what `plan` learns without creating a run

`plan` sends nothing that writes (C1). It calls `lookup` only, as many times as K10's limits require.

Request:

```json
{
  "hashes": ["<blake3>", "…"],
  "entries": [{"path": "data/a.csv", "hash": "<blake3>"}],
  "users": ["jdoe", "…"]
}
```

Response 200:

```json
{
  "have": ["<blake3>"],
  "imported": [{"path": "data/a.csv", "hash": "<blake3>"}],
  "unmatched_users": ["jdoe"],
  "compression": "zstd",
  "migrated": false
}
```

| Key | Meaning |
| --- | --- |
| `have` | The subset of `hashes` the project has content for: a completed upload with that blob hash in the project (the rule `manifest` already uses to skip uploads) |
| `imported` | The subset of `entries` that already have a completed *imported* upload in the project, from any run. These are "already present" (A2) and will be skipped. A live add of the same pair does not count. |
| `unmatched_users` | The subset of `users` that are not exactly (case-sensitive) a directory username (A4) |
| `compression` | The project's compression (as `UploadManifestResponse.compression`) |
| `migrated` | Whether finalize has already succeeded for this project |

Each list may be empty or absent (absent = empty). `run` and `finalize` call `lookup` too, for the same answers.

### K6 — Start run

Request:

```json
{
  "client_version": "dvs-migrate 0.1.0",
  "walk": "full",
  "totals": { "…": "K12 totals object, as computed by the walk before uploading" }
}
```

`client_version` is in the body, not taken from `User-Agent`, because the binary reuses dvs2's HTTP agent, which sends `dvs/<version>`.

Response 201:

```json
{"run_id": "<uuid v4>", "started_at": "2026-11-05T14:02:11.123Z", "compression": "zstd"}
```

| Status | Meaning |
| --- | --- |
| 201 | Run opened. The server picks the id. |
| 409 `project is already migrated` | Finalize has succeeded; no more runs |

A run is **open** until its report is stored (**reported**) or a later start run in the same project closes it (**interrupted**). Starting a run closes every open run in the project as interrupted. A run's A6 event ("migration run by <admin>") is written when it closes, as reported or interrupted, only if it uploaded content or recorded imports (I6), with the totals of what it actually did.

### K7 — Manifest

The binary sends **every** entry the walk found, in chunks (K10), whether or not it was imported before and whether or not its content is available. The server:

1. Stores each entry as a report entry of the run (D9).
2. Skips entries already imported (K5 `imported`); records the rest as imported upload rows (A3). Imports are grouped into one operation per (run, `commit.id`) (A5), created by the server on first sight, with `git_commit` = `commit.id`, `git_branch` and `message` null, `created_by` the issuing admin. Each entry's sidecar `message` stays on its own row.
3. Completes at once any new import whose content the project already has (as `record_manifest` does). The others stay pending until their blob arrives; a pending import is not an imported version and never shows (as today's declared-but-not-arrived rows).

Request:

```json
{
  "run_id": "<uuid>",
  "entries": [
    {
      "path": "data/a.csv",
      "hash": "<blake3>",
      "size": 1048576,
      "shape": "dvs1",
      "sidecar_path": "data/a.csv.dvs",
      "storage_path": "/data/shared/dvs-store",
      "sidecar_user": "jdoe",
      "recorded_user": "jdoe",
      "recorded_user_source": "sidecar",
      "recorded_at": "2023-04-05T14:11:12.345Z",
      "recorded_at_source": "sidecar",
      "message": "first pull of the trial data",
      "commit": {
        "id": "<40-hex sha>",
        "author_name": "Jane Doe",
        "author_email": "jdoe@example.com",
        "author_date": "2023-04-05T14:20:00Z"
      },
      "user_mismatch": false
    }
  ]
}
```

Entry fields (F3, D5, A3, A4):

| Field | Type | Null? | Holds |
| --- | --- | --- | --- |
| `path` | string | no | Data file path in the project |
| `hash` | string | no | blake3 from the sidecar (`blake3_checksum` or `hashes.blake3`) |
| `size` | integer | no | Size from the sidecar (`file_size_bytes`, `size`), uncompressed bytes |
| `shape` | `"dvs1-v0.0.1"` \| `"dvs1"` \| `"dvs2"` | no | The introducing commit's sidecar shape (D4). `dvs1` is v0.0.2 and later. |
| `sidecar_path` | string | no | The sidecar's path at the introducing commit: `<path>.dvs` (dvs1) or `<metadata_folder>/<path>.dvs` (dvs2) |
| `storage_path` | string | yes | The storage root the content is looked up in (F4), after `--storage-map`; null when the config at the introducing commit could not be read |
| `sidecar_user` | string | yes | `saved_by` (dvs1) or `created_by` (dvs2) as written; null if absent or empty |
| `recorded_user` | string | no | The user the import is shown as (A9): dvs2-local `audit.log.jsonl` user for the same (path, hash), else `sidecar_user`, else the commit's `author_name` (D5) |
| `recorded_user_source` | `"audit_log"` \| `"sidecar"` \| `"commit"` | no | Where `recorded_user` came from |
| `recorded_at` | RFC 3339 | no | The time the import is shown at: audit-log time, else sidecar `time_stamp`/`add_time`, else `commit.author_date` |
| `recorded_at_source` | `"audit_log"` \| `"sidecar"` \| `"commit"` | no | Where `recorded_at` came from. A sidecar time that does not parse as RFC 3339 falls through to the commit. |
| `message` | string | yes | The sidecar's `message`; null when absent or empty (dvs1 always writes the key, often `""`) |
| `commit.id` | string | no | Full SHA-1 of the introducing commit (F3) |
| `commit.author_name` | string | no | Git author name |
| `commit.author_email` | string | no | Git author email, as recorded (may be empty) |
| `commit.author_date` | RFC 3339 | no | Git author date, converted to UTC |
| `user_mismatch` | boolean | no | `sidecar_user` is non-null and equals, case-insensitively, neither `commit.author_name` nor the local part of `commit.author_email` (D5). False when `sidecar_user` is null. |

The server matches `recorded_user` to a directory username itself (A4); the manifest does not carry the match.

Response 200 (the existing `UploadManifestResponse` keys, plus two counts):

```json
{"missing": ["<blake3>"], "compression": "zstd", "recorded": 120, "already_imported": 3880}
```

| Key | Meaning |
| --- | --- |
| `missing` | Distinct hashes among this call's newly recorded entries that the project has no content for: the only ones to upload |
| `recorded` | Entries in this call recorded as new imports (completed or pending) |
| `already_imported` | Entries in this call skipped as already imported |

| Status | Meaning |
| --- | --- |
| 200 | Recorded. Sending the same entries again in the same run changes nothing (I6). |
| 400 `too many entries` | Over K10's entry limit |
| 423 | Locked (K3) |

### K8 — Blob upload

`PUT /api/{name}/migration/blobs/{hash}?run_id=<uuid>`, body the uncompressed bytes (A7). Response body is the existing `UploadResponse`: `{"stored_size": 123, "compression": "zstd"}`. Arriving content completes every pending import in the project with that hash.

| Status | Body starts with | Meaning |
| --- | --- | --- |
| 201 | — | Stored now |
| 200 | — | The project already had it |
| 400 | `content hash mismatch` (existing `store_error` text) | Bytes do not hash to `{hash}` (I3). The binary records a `corrupt` problem. |
| 409 | `<hash> is not in the manifest of migration run <id>` | No manifest entry in this run has this hash |
| 413 | `the file is over the server limit` (existing) | Over `max_blob_bytes`. The binary records an `upload_refused` problem. |
| 423 | (K3) | Locked |

### K9 — Finalize

Request:

```json
{"run_id": "<uuid>", "accepted_missing": ["<blake3>"]}
```

`run_id` is the run the binary's fresh `plan` was compared against: the project's latest run. `accepted_missing` is the run's missing list (K12) as the admin accepted it.

Response 200, both when it succeeds and when it refuses:

```json
{"migrated": true, "migrated_at": "2026-11-05T16:00:00Z", "refusals": []}
```

```json
{"migrated": false, "migrated_at": null, "refusals": [
  {"reason": "missing_not_accepted", "not_accepted": ["<blake3>"], "not_missing": []}
]}
```

On success the server marks the project migrated, revokes the key (A1) and writes the "project migrated by <admin>" event (A6). Refusal reasons, a closed enum; the binary adds its own (client-side) refusals to the same list in its report (K12 `finalize`):

| `reason` | Checked by | Extra fields | Refused when |
| --- | --- | --- | --- |
| `to_upload` | client | `hashes`, `entries` (counts) | The fresh plan finds a hash to upload, or an entry whose content is on the server but is not yet imported |
| `partial_walk` | client and server | — | The fresh plan's walk is partial (client), or the run's reported walk is partial (server) (Q2) |
| `storage_writable` | client | `paths` (list) | A storage path in scope is writable by the user running the binary, by an access check that creates nothing (I1) |
| `missing_not_accepted` | client and server | `not_accepted`, `not_missing` (hash lists) | The admin declined the prompt (client), or `accepted_missing` is not exactly the run's missing list (server) |
| `not_latest_run` | server | `latest_run_id` | `run_id` is not the project's latest run |
| `run_not_reported` | server | — | The run is open or interrupted |
| `run_changed_something` | server | `uploaded`, `imported` (counts) | The run stored content or recorded imports: the last run must find nothing new (D11) |
| `already_migrated` | server | `migrated_at` | Finalize already succeeded |

The binary only calls finalize when it has no client-side refusals. The server's checks use its own records, not the report's claims, except `partial_walk` and the missing list, which only the report holds.

### K10 — Batching

| Limit | Value | Applies to |
| --- | --- | --- |
| JSON body | 64 MiB (the existing `MANIFEST_MAX_LEN`) | `lookup`, `manifest`, `report` |
| Entries per call | 10 000 | `manifest` `entries`; each of `lookup`'s `hashes`, `entries`, `users` |

Over the entry limit is 400 `too many entries: <n>, at most 10000 per call`. The binary chunks to stay under both limits (an entry with a long message can be large), in any order: the server does not depend on chunk order or on all of a commit's entries arriving in one call.

### K11 — Order of calls

`plan`: `lookup`* only. `run`: `lookup`* → `runs` → `manifest`* → `blobs`* (the `missing` hashes it can read locally) → `report`. `finalize`: `lookup`* (as `plan`) → `finalize`. A run cut short before `report` is closed as interrupted by the next `runs` call (D10).

### K12 — The report document

One document, written by every `dvs-migrate` command: printed with `--json`, written to the report file, and (for `run`) sent to `report`. It is dvs2's `--json` envelope (`dvs-cli/src/output.rs`: `v`, `command`, `outcome`, `dry_run`, `error`, `started_at`, `finished_at`, `duration_ms`) with the F5 parts added.

```json
{
  "v": 1,
  "command": "run",
  "outcome": "complete",
  "dry_run": false,
  "error": null,
  "started_at": "2026-11-05T14:02:10.001Z",
  "finished_at": "2026-11-05T14:09:44.512Z",
  "duration_ms": 454511,
  "run_id": "<uuid>",
  "scope": {
    "source": "git@github.com:org/proj.git",
    "walk": "full",
    "partial_reasons": [],
    "refs": ["refs/heads/main", "refs/remotes/origin/dev", "refs/tags/v1"],
    "commits": 812,
    "storage": [
      {"path": "/old/store", "shape": "dvs1", "remapped_to": "/data/shared/dvs-store"}
    ]
  },
  "totals": {
    "sidecars": {"dvs1-v0.0.1": 40, "dvs1": 900, "dvs2": 0},
    "entries": 4000,
    "unique_hashes": 950,
    "bytes": 52428800000,
    "on_server": 10,
    "to_upload": 930,
    "missing": 10,
    "already_imported": 0,
    "user_mismatches": 7,
    "uploaded": 928,
    "imported": 4000
  },
  "entries": [ { "…": "K7 entry", "content": "uploaded", "already_imported": false } ],
  "problems": [ {"kind": "missing", "…": "…"} ],
  "finalize": null
}
```

| Key | Type | Meaning |
| --- | --- | --- |
| `command` | `"plan"` \| `"run"` \| `"finalize"` | |
| `outcome` | `"complete"` \| `"partial"` \| `"failed"` \| `"cancelled"` | dvs2's `RunOutcome`; `partial` when any problem was recorded |
| `dry_run` | boolean | True for `plan` (F5: a dry run is the same report) |
| `run_id` | uuid \| null | Null for `plan` and `finalize` |
| `scope.source` | string | The repo argument as given (Q2) |
| `scope.walk` | `"full"` \| `"partial"` | Q2 |
| `scope.partial_reasons` | list | Empty for a full walk. Each `{"kind": "unreachable_remote", "remote": "<name>", "url": "<url>", "message": "…"}` or `{"kind": "shallow"}` |
| `scope.refs` | list of strings | Every ref walked, full names (D3) |
| `scope.commits` | integer | Distinct commits visited (F1) |
| `scope.storage` | list | Every storage path seen in history (F4): `path` as in the config, `shape` `"dvs1"` \| `"dvs2"`, `remapped_to` string or null |
| `totals.sidecars` | object | Distinct sidecar git blobs parsed, by shape (F2) |
| `totals.entries` | integer | Distinct (path, hash) |
| `totals.unique_hashes` | integer | Distinct hashes; always `on_server + to_upload + missing` (I2) |
| `totals.bytes` | integer | Sum of `size` over unique hashes |
| `totals.on_server` | integer | Hashes the project already had before this command |
| `totals.to_upload` | integer | Hashes the project lacks that are present in local storage |
| `totals.missing` | integer | Hashes the project lacks that are absent locally, or (after `run`) were refused as corrupt or too large: the **missing list** that finalize accepts is these hashes |
| `totals.already_imported` | integer | Entries already imported before this command |
| `totals.user_mismatches` | integer | Entries with `user_mismatch` (D5) |
| `totals.uploaded`, `totals.imported` | integer \| null | What this run stored and newly recorded; null for `plan` and `finalize` |
| `entries[]` | list | Every K7 entry, plus `content`: `"on_server"` \| `"to_upload"` \| `"uploaded"` \| `"missing"` \| `"corrupt"` \| `"refused"`, and `already_imported`: boolean |
| `problems[]` | list | K13 |
| `finalize` | object \| null | `finalize` only: the K9 response, with the client-side refusals added to `refusals` |

What the server stores (D9): `scope`, `totals`, `problems` and `outcome`/`started_at`/`finished_at` from the `report` call, as records of the run; `entries` from the run's `manifest` calls, which carry every entry (K7), with `content` and `already_imported` derived on the server. So the `report` call sends the document **without `entries`** (the only difference); a full copy for download (U1) is rebuilt from the records. `v`, `command`, `dry_run`, `error` and `finalize` are not stored.

`report` request: the document as above, `command` `"run"`, `run_id` the open run. Response 200 `{"run_id": "<uuid>", "entries": 4000, "problems": 12}` (counts stored). 400 `invalid report` if `command` is not `"run"` or `entries` is present; 404/409 as K4.

### K13 — Problem kinds

A closed enum on `kind`. F5's five, plus two this contract needs (`upload_refused`, `unreadable_config`).

| `kind` | Fields | Recorded when |
| --- | --- | --- |
| `missing` | `hash`, `size`, `paths` (list), `looked_at` (string \| null: the file path tried) | The project lacks the hash and no file is at `<storage>/<hash[..2]>/<hash[2..]>` (D8) |
| `corrupt` | `hash`, `paths`, `looked_at`, `message` (the server's 400 body) | `run` only: the server refused the bytes as a hash mismatch (I3) |
| `upload_refused` | `hash`, `paths`, `looked_at`, `status` (integer), `message` | `run` only: the server refused the bytes for another reason (413 over the size limit) |
| `size_mismatch` | `hash`, `recorded_sizes` (list of integers), `actual_size` (integer \| null), `looked_at` | The local file's size differs from the sidecar's, or sidecars disagree on the size of one hash. Content is still uploaded; the server's hash check decides. |
| `unrecognized` | `sidecar_path`, `commit` (first commit seen), `blob` (git blob id), `message` | A `.dvs` file no shape matches (D4) |
| `unreadable_config` | `config_path` (`dvs.yaml` or `dvs.toml`), `commit`, `message` | A commit's config does not parse, so its entries have `storage_path` null and their hashes count as missing unless the server has them |
| `unmatched_user` | `user`, `entries` (count) | A `recorded_user` that is not a directory username (A4, from K5 `unmatched_users`) |

A hash appears in at most one of `missing`, `corrupt`, `upload_refused`.

### K14 — Shared Rust surface (`dvs::legacy`, P1)

A new public module `dvs::legacy` (the P1 path `dvs/src/legacy/`; no existing module fits — `files` is private and dvs2-only). P4 and P5 use only these; D13 holds because nothing else in `dvs` calls them.

| Name | Behavior |
| --- | --- |
| `SidecarShape` | Enum `Dvs1V001`, `Dvs1`, `Dvs2`; serializes as `"dvs1-v0.0.1"`, `"dvs1"`, `"dvs2"` (K7 `shape`) |
| `Sidecar` | Enum: `Dvs1(Dvs1Sidecar)` or `Dvs2(FileMetadata)` |
| `Dvs1Sidecar` | One struct for both dvs1 shapes: `shape`, `blake3`, `size`, `recorded_at` (the raw `time_stamp`/`add_time` string), `message`, `saved_by` |
| `classify_sidecar(bytes) -> Result<Sidecar, Unrecognized>` | Parses a `.dvs` file's JSON and picks the one shape whose required keys are present with the right types: v0.0.1 `blake3_checksum`, `file_size_bytes`, `time_stamp`, `message`, `saved_by`; v0.0.2+ the same with `size`, `add_time`; dvs2 whatever `FileMetadata` deserializes. Extra keys are ignored, as each tool's own reader ignores them. None or more than one matching is `Unrecognized` with a message (D4). |
| `Dvs1Config` | `storage_dir`, `permissions: Option<i32>`, `group: Option<String>`, as dvs1's `dvs.yaml` |
| `Dvs1Config::from_yaml(&str)` | Parses `dvs.yaml` |
| `DVS1_CONFIG_FILE` | `"dvs.yaml"` |
| `Dvs1Sidecar::to_metadata(&self) -> Result<FileMetadata>` | The dvs2 sidecar `dvs migrate` writes: `hashes.blake3`, `size`, `created_by` = `saved_by`, `add_time` parsed from `recorded_at`, `compression` none (D15: the store holds uncompressed blobs), `message` none when empty |
| `sidecar_data_path(path) -> Option<ProjectPath>` | `data/a.csv.dvs` → `data/a.csv`; None if it does not end in `.dvs` |
| `blob_path(storage_dir, hash) -> PathBuf` | `<storage_dir>/<hash[..2]>/<hash[2..]>`, the layout dvs1 and dvs2 local share (`LocalBackend::hash_to_path`) |

Existing items P5 also uses, unchanged: `dvs::FileMetadata`, `dvs::config::Config` (deserialized from a commit's `dvs.toml` bytes with `toml`; `metadata_folder_name()`, `backend()` for the local path), `dvs::audit::parse_audit_log` (D5), `dvs::Compression::decompress` (A7), and the `dvs::backends::server` client helpers (`join_url`, `status_error` and friends would need to become `pub` to be used from another crate — they are `pub(crate)` today).

## Resolutions from implementation review (R1)

### R1 — `access` returns the project's compression

`GET /api/{name}/access` gains a `compression` field (`"zstd"` | `"none"`), alongside `access`, `user` and `groups`. Any project member can read it. `dvs migrate server` reads the compression from this call instead of sending an empty manifest, so a read-only member can use the recovery path (D17/W4) and no empty operation reaches the audit log. dvs2 treats the field as optional so it still works against older servers, and falls back to the empty-manifest probe only when the field is absent.

This supersedes the P4 implementation choice of always probing with an empty manifest. It adds one field to an existing response.

*Decided — Wes, 2026-11-05.*

### R2 — A run "changed something" only when content arrived

An import whose content is still missing stays pending and is re-recorded by every run, so it does not count as a change. For finalize's `run_changed_something` check, the A6 run event and the run totals, a run changed something only if it uploaded content or completed an import. The manifest's `recorded` count still includes pending imports (K7). Without this, finalize could never succeed while hashes are missing, which D8 allows.

*Decided — my call, reversible (resolves a conflict between K7/K9/A6 and D8).*

### R3 — `--upload-missing` takes the storage path explicitly

`dvs migrate server --upload-missing` accepts `--storage <dir>`. It defaults to `storage_dir` from `dvs.yaml` (or the dvs2-local path being replaced) when that config is still present in this run. On a later run, after `dvs.yaml` is gone, `--storage` is required for uploading. When hashes are missing but found in local storage, the hint prints the exact rerun command, including the path. This supersedes D18's implicit "local storage from the config being replaced" and makes `--upload-missing` usable after the converting run.

*Decided — Wes, 2026-11-05.*

### R4 — No "replace the backend" option in v1

When `dvs.toml` already points at a different server or a different local store, `dvs migrate` always refuses, giving the reason. Switching servers is not part of migration. This supersedes C6's "unless told to replace the backend".

*Decided — Wes, 2026-11-05.*

### R5 — A path seen twice in one commit gets a second operation

A5 groups a run's imports into one operation per introducing commit. The server already allows only one row per (operation, path). When the same path arrives with a second hash under the same commit, which only happens when one file has both a dvs1 and a dvs2 sidecar (D20), that entry goes into an additional operation for the same (run, commit) instead of failing the manifest.

*Decided — my call, reversible.*

### R6 — Detecting a stored blob's compression (corrects D15)

D15 said "dvs2 local detects per-blob compression". On main it does not: a deduped `add` recorded the requested compression, not the stored one. `fix/dedup-stored-compression` (`existing_compression` in the local backend: zstd magic bytes, then a hash check) fixes that and is merged into `feat/migration` (a821ae6). With it, a dvs1 store reused by `dvs migrate local` stays correct when later dvs2 adds dedupe onto its uncompressed blobs. `dvs migrate server --storage` uses the same detection for blobs whose compression this run doesn't know, through a shared helper. It does not guess from size.

*Decided — Wes, 2026-11-05 (merge); helper shape: my call, reversible.*

### R7 — Corrupt or refused content counts as missing at finalize

A fresh plan can't tell a corrupt local blob from a good one without re-hashing (D7). So finalize counts hashes that the last run reported as `corrupt` or `upload_refused` as missing, not as left to upload. They then go through the D8 accept-the-missing-list step instead of blocking finalize forever.

*Decided — my call, reversible.*

### R8 — `lookup` returns the latest run; finalize needs no report file

`lookup` also returns the project's latest run: its id, state (open / reported / interrupted) and missing hashes (the run's `missing`, `corrupt` and `upload_refused` problems). `dvs-migrate finalize` closes that run and works from any machine that has the key. There is no `--run-report` flag. The change only adds fields to the `lookup` response.

*Decided — Wes, 2026-11-05.*

Wire shape for R8. The `lookup` response gains `"latest_run": null` or `{"id": "<uuid>", "state": "open" | "reported" | "interrupted", "missing": ["<hash>", …]}`. `missing` holds the distinct hashes of that run's `missing`, `corrupt` and `upload_refused` problems, the same list finalize compares `accepted_missing` against.

### R9 — Finalize: content missing now, and content the last run couldn't read

Finalize compares its fresh plan with the latest run's missing list. A hash missing now but not in that list makes finalize refuse with `missing_not_accepted`, naming those hashes; the admin runs again first. A hash the last run could not read locally (a decode or I/O error) counts as missing at finalize, like R7's corrupt and refused content. It does not block finalize as "to upload".

*Decided — my call, reversible.*

### R10 — Values the server would refuse, credentials, cleanup, compression

- `recorded_user` falls back from the sidecar or audit log, to the commit author name, then the author email, then the literal `unknown`. It is never empty.
- NUL characters are stripped from every text value sent to the server.
- Credentials (URL userinfo) are redacted from the repo source, remote URLs and git error text before printing, writing the report or sending it.
- On Ctrl-C the binary removes its temp mirror.
- dvs2-local blobs are decoded using the stored-compression detection from R6, not the sidecar's `compression` field alone. A7 still holds: the server always receives uncompressed bytes.

*Decided — my call, reversible.*

### R11 — Migration page detail, manual revoke, frozen reports

- For the latest report, the Migration page shows its problems (kind, hash, path, message) and the entries flagged with a user mismatch (D5). Each list is capped on the page with an "and N more in the download" line. The page also shows the report's scope (refs walked, storage paths, partial walk).
- An admin can revoke a key by hand from the page (reason `revoked`), alongside reissue and finalize. This adds to A1.
- A run's report is frozen when it is stored: a later run that supplies the missing content does not change what an earlier run's download says (D9).
- A report whose `totals` or `scope` has the wrong types is refused with 400 `invalid report`.
- The response that shows a new key is sent with `Cache-Control: no-store`.

*Decided — my call, reversible.*

### R12 — End-to-end review: compression, guard scope, permissions

- `dvs migrate server --upload-missing` always detects a local blob's compression (R6 helper), even when this run "knows" it from a sidecar. Pre-fix dedupe could mislabel a blob in either direction.
- The D16 guard also covers `dvs init` and `dvs attach`. Only `dvs migrate`, and `dvs login <url>` with an explicit URL, run in a dvs1 checkout.
- dvs1 `permissions` do not carry over to dvs2 local storage, which has no such setting; `dvs migrate local` warns about it. This amends D15, which said "`permissions`/`group` carry over". `group` still does.
- A `--storage-map` entry that matches no storage path in history produces a warning in the report and on the console.
- A dvs2-shaped sidecar found outside a dvs2 metadata folder (for example a commit with `.dvs/` but no `dvs.toml`) is reported as `unrecognized`, not imported.

*Decided — my call, reversible.*

K13 gains the problem kind `unused_storage_map` (`storage_path`, `remapped_to`), used for the storage-map warning above.

## Resolutions from iteration (R13–R14)

### R13 — No finalize

A migration has no finalize step and no "migrated" state. It is a run, repeated as often as needed. Each run that changes anything records "migration run by <admin>" (A6), and that is the record that the project was migrated.

Withdrawn:
- C3 (`dvs-migrate finalize`), K9 (the finalize endpoint and its refusals), and R7–R9.
- R8's `latest_run` field on `lookup`.
- D8's accept-the-missing-list step. Missing content stays visible in each run's report and in the Migration page's current state.
- D11's refusal while storage is writable. `run` ends with a warning naming the command to make storage read-only; making it read-only stays the admin's step.
- On the server: `projects.migrated_at`, the `migrated` project event, revoke-on-finalize, and the 409 for starting a run on a migrated project.

Kept: migrate keys (8 hours after first use, reissue, manual revoke), runs, imports, reports, and the current-state Migration page.

*Decided — Wes, 2026-11-06.*

### R14 — dvs-migrate output

- No files are written: no report JSON and no `.txt` summary. `--report` is removed. The server keeps every run's report, downloadable from the Migration page.
- `--json` prints the report document (K12, unchanged apart from R13) to stdout and nothing else.
- Without `--json`, stdout is a short summary: a header line (repo → server and project), one walk line (commits, refs, full or partial), one totals line (versions, already on server, uploaded, missing), then a "Missing" list (path, short hash, reason; capped, with "and N more"), counts of the other problem kinds, and a pointer to the Migration page. Warnings (unused storage map, storage still writable after `run`) go to stderr.
- `plan` uses the same summary and says nothing was sent.

*Decided — Wes, 2026-11-06 (summary layout: my call, reversible).*

### R15 — "migrated", not "imported"

Everywhere a version brought over by a migration run is named, it is "migrated": UI text, hover text, CSV, JSON wire fields, the database and the code. The noun "an import" becomes "a migrated version". Wire and storage renames:

| Where | Was | Now |
|---|---|---|
| `lookup` response | `imported` | `migrated` |
| manifest response | `already_imported` | `already_migrated` |
| report `totals` | `imported` | `migrated` |
| report entry | `already_imported` | `already_migrated` |
| `migration_runs` column, `migration_run` event detail | `imported` | `migrated` |
| `migration_entries` column | `already_imported` | `already_migrated` |
| audit views, CSV column | `imported`, `run_imported` | `migrated`, `run_migrated` |

UI tag: "migrated". Hover: "Who and when as recorded in the project's history, brought over by a migration run". This supersedes earlier text that says "imported". 0002 is edited in place because it is unreleased.

*Decided — Wes, 2026-11-06.*

## Resolutions from review (R16–R24)

The key and the separate binary are the friction: only an admin with a key from that project's Migration page can run a migration, and no user has `dvs-migrate`. These resolutions add semantics around re-runs and long or failed runs, report on orphaned content, and cover devious, the Go predecessor of dvs1.

### R16 — Say what is already there

`plan` and `run` open with the project's migration history: the number of migrated versions, the number of runs, and the last run's admin and time, for example "42 versions migrated in 2 runs (last: alice, 2026-11-07); this walk adds 3". It is information only. A project with no migrated versions says nothing.

`lookup` (K5) returns this as `previous: {runs, migrated, last_run_by, last_run_at}`, or `null` when there have been no runs.

*Decided — Wes, 2026-11-09.*

### R17 — A different source is refused

When the project already has migrated versions and none of the walk's commits are among the commits that earlier runs recorded for their entries, the walk looks like a different repo. `plan` warns. `run` refuses with exit status 2, naming the earlier runs' source, unless `--other-source` is passed. A run started with `--other-source` records it in its start-run request (`other_source: true`), and the run's event shows it.

`lookup` takes the walk's commit ids (`commits`, chunked like `hashes`, K10) and answers `known_commits`, how many of them earlier runs recorded. The check is "zero known out of a non-zero walk".

*Decided — Wes, 2026-11-09 (flag name: my call, reversible).*

### R18 — Timeouts and retries

- Every request has a connect timeout (30 s) and a read timeout (5 min, since a large blob upload streams).
- A transport error, a timeout or a 5xx on `lookup`, `runs`, `manifest`, `blobs` or `report` is retried up to 5 times with exponential backoff (1, 2, 4, 8, 16 s). Each of these calls is safe to repeat (D10, I6). A 4xx is not retried.
- When the retries run out, the run ends with exit status 1 and "interrupted after N of M versions (X GB); run again to resume" on stderr. The run stays open on the server (R19).

*Decided — Wes, 2026-11-09 (values: my call, reversible).*

### R19 — An active run holds the project

A run is **active** while it is open and its last request arrived less than 15 minutes ago. Every migration call with the run's id updates its `last_activity_at`, at arrival and at completion.

- **The key stays alive.** While its run is active, a migrate key does not expire. The 8-hour limit (D2) still applies, counted from the later of first use and the active run's last activity.
- **The key is held.** While a run is active, the Migration page does not reissue or revoke the key; it shows "Run in progress by alice since 14:02 (last activity 14:31)". An "End this run" control, behind a confirmation, closes the run as interrupted and revokes the key. A request to reissue or revoke while a run is active is 409 `a migration run is in progress`.
- A run with no request for 15 minutes is **abandoned**. It stays open until the next start run closes it (R20). An abandoned run does not hold the key.

There is one key per project (D2), so two admins running at once share it; R20 stops them colliding.

A single blob upload longer than 15 minutes without another request lets the run count as abandoned. This is accepted. The window is a default.

*Decided — Wes, 2026-11-09 (15 minutes: my call, reversible).*

### R20 — One run never interrupts another

Supersedes K6's "starting a run closes every open run".

- Starting a run while another run is active is 409 `a migration run is in progress: started by <admin> at <time>, last activity <time>`. `dvs-migrate run` prints it and exits with status 2.
- `--take-over` sends `take_over: true`. The server then closes the active run as interrupted, records "taken over by <admin>" on it, and opens the new run.
- Abandoned runs (R19) are closed as interrupted by the next start run without the flag.

*Decided — Wes, 2026-11-09 (flag name: my call, reversible).*

### R21 — Show what an interrupted run did

- The start-run response (K6) gains `closed`: for each run it closed, `{run_id, started_by, started_at, last_activity_at, migrated, uploaded_bytes, taken_over}`. `run` prints one line per closed run before its own summary, for example "previous run (alice, 14:02) was interrupted after 1,204 versions (38 GB); resuming".
- On the Migration page, an interrupted run's row shows the same counts and, when it was taken over, by whom.

The counts come from what the server recorded under that run: its migrated versions and the bytes of the blobs it uploaded.

*Decided — Wes, 2026-11-09.*

### R22 — How much of local storage the migration covers

The summary (R14) gains one line, for example: "local storage: 1.2 TB in 3 stores; history references 0.9 TB; 0.9 TB on the server after this run". `plan` says "would be on the server".

The report's `totals` (K12) gain `storage_bytes` (every blob file in the stores the walk saw), `referenced_bytes` (the stored size of each hash in history found in local storage, once per hash) and, for `run`, `on_server_bytes`. They are computed from file sizes, without reading content.

*Decided — Wes, 2026-11-09.*

### R23 — `dvs-migrate orphans` lists what history does not reference

`dvs-migrate orphans <repo>... [--storage-map OLD=NEW]... [--json]` walks each repo as `plan` does, then lists every blob in the stores those walks saw that no commit in any of them references. It is read-only: it never deletes, and it needs no key and no server.

- For each store: the path, and for each orphan its hash, size and modification time. Then a total.
- Files in a store that are not laid out as blobs (any layout in R24 or the dvs2 layout) are listed as "other files", never as orphans.
- A store can be shared by several projects. An orphan is unreferenced only by the repos given. The output says so ("orphaned with respect to these 2 repos") and, when a store's blobs are mostly orphans, warns that the store may be shared.
- `--json` prints `{repos, stores: [{path, orphans: [{hash, size, modified}], other_files, orphan_bytes}], orphan_bytes}`.

*Decided — Wes, 2026-11-09.*

### R24 — devious projects

devious is the Go dvs that came before the dvs1 R package. Its formats are in history wherever a project used it. Its last format (the "normalize fields" commit, tagged v0.3.0) is the dvs1 shape and is already covered; the earlier ones are not.

| devious releases | Dates | Sidecar | Sidecar keys (JSON) | Config | Store layout | Hash |
|---|---|---|---|---|---|---|
| v0.1–v0.4 | 2023-08-03 – 08-08 | `<file>.dvsmeta` | `FileHash` (Go field names) | `.dvs.yaml`, `storage-dir` | `<storage>/<hash>.dvsfile` | blake3 of the file's **path** |
| v0.4.1–v0.7.2 | 2023-08-08 – 08-14 | `<file>.dvsmeta` | `FileHash`, `FileSize`, `Timestamp`, `Message`, `User` | `.dvs.yaml`, `storage-dir` | `<storage>/<hash>.dvsfile` | blake3 of content |
| v1.0–v1.2.1 | 2023-08-15 – 08-24 | `<file>.dvsmeta` | `blake3_checksum`, `file_size_bytes`, `timestamp`, `message`, `user` | `.dvs.yaml`, `storage-dir` | `<storage>/<hash>.dvsfile` | blake3 of content |
| v2.0.0–v2.0.3 | 2023-10-11 – 2024-08-01 | `<file>.dvs` | `blake3_checksum`, `file_size_bytes`, `timestamp`, `message`, `saved_by` | `dvs.yaml`, `storage-dir` | `<storage>/<hh>/<rest>` | blake3 of content |
| normalize fields (v0.3.0) | 2024-08-15 | `<file>.dvs` | dvs1 shape (`size`, `add_time`, `saved_by`) | `dvs.yaml`, `storage_dir` | `<storage>/<hh>/<rest>` | blake3 of content |

devious v2's own `dvs migrate` renames `.dvsmeta` to `.dvs`, rewrites the sidecar, renames `.dvs.yaml`, and moves `<hash>.dvsfile` blobs into `<hh>/<rest>`. A store can therefore hold either layout, and a commit from v1 can name a blob that now sits in the nested layout.

- **Shapes.** `SidecarShape` gains `devious-v0` (Go field names), `devious-v1` (`timestamp`, `user`) and `devious-v2` (`timestamp`, `saved_by`). `classify_sidecar` (D4, D21) recognizes them by their required keys like the others. `devious-v0` requires only `FileHash`; its other keys are optional. The server's `shape` CHECK gains the three names (0002 is edited in place: unreleased).
- **Finding sidecars.** A sidecar is a `.dvs` or `.dvsmeta` file; its data path drops that suffix.
- **Finding the config.** Each commit's config is `dvs.yaml` or `.dvs.yaml` at the repo root, with `storage_dir` or `storage-dir`. `Dvs1Config` reads all four combinations.
- **Finding content.** `find_blob` tries `<storage>/<hh>/<rest>`, then `<storage>/<hash>.dvsfile`, then `<storage>/<hash>` (a devious migrate interrupted between its two steps). devious stored content uncompressed; compression is detected from the blob as for any store (R6).
- **Provenance.** `user`/`saved_by` and `timestamp` map onto the sidecar's user and time (D5); devious wrote the OS username of whoever added the file. `devious-v0` without `User` or `Timestamp` falls through to the commit.
- **Path hashes.** v0.1–v0.4 recorded a hash of the path, which no content matches, and the file shape cannot tell them from v0.4.1–v0.7.2. Such content fails the hash check and is reported as `corrupt` (I3), with the hint "devious v0.4 or earlier hashed the path, not the content". Nothing is uploaded for it.
- **`dvs migrate` (users).** A checkout whose working tree has devious sidecars or a devious config is treated like a dvs1 checkout (D16, W2, W6). Its sidecars are converted to dvs2, and its `.dvsmeta` sidecars and `.dvs.yaml` are removed, as dvs1's are.

*Decided — Wes, 2026-11-09 (shape names, the `dvs migrate` coverage: my call, reversible).*
