# Side-by-side dataset versions — Spec v1

Source doc: https://claude.ai/artifact/RYU4yr33QpVtim3eppZGA9 (working copy; this file is now the authority).

This spec is the authority for the feature. Implementers follow it and do not redesign; later
decisions are appended as new sections that say what they supersede.

How a dvs2 user gets an older version of a tracked file next to the current one to compare them,
replacing the dvs1 trick of hand-writing a `.dvs` file with an old hash.

## B — Problem

On dvs-server a hand-written `.dvs` file with an old hash fails to download, because the server
only serves a blob for a (path, hash) pair that was uploaded.

### B1 — dvs1: the trick works because storage is hash-only

The user copies `data.csv.dvs` to `data_v1.csv.dvs`, pastes an old hash into it, then runs
`dvs get data_v1.csv`. dvs1 storage is content-addressed, so any hash it holds comes back under any name.

### B2 — dvs2 local backend: probably still works

`LocalBackend::retrieve` resolves the blob by hash and ignores the path. A dvs2 sidecar with a
pasted hash should still `get`. Not tried end to end.

### B3 — dvs-server: the fabricated path gets a 404

Every download is recorded against an upload: `record_download` looks for an upload row with this
project, hash **and path**. `data_v1.csv` was never uploaded, so it returns `UnknownUpload` → 404,
and the CLI reports the file as missing from storage.

### B4 — dvs2 has no other way to see two versions

A tracked path has one working copy. `dvs history <file>` lists past hashes but nothing fetches one
of them. The only route today is `git checkout <old>` + `dvs get`, which replaces the current copy
instead of sitting beside it.

## O — Options

O2 is chosen (Q1): the user gets a comparison copy without a fake tracked file, and the server
audit stays honest. O3 stays possible later if a comparison copy ever needs sharing.

| ID | Shape | What the user runs | Server change | Audit truth | Cost |
| --- | --- | --- | --- | --- | --- |
| O1 | Server falls back to any upload of the hash | Same dvs1 trick, unchanged | Download accepts a path with no upload of that hash | Records a download of a path that was never uploaded | Smallest; keeps fake sidecars in git forever |
| O2 | Fetch an old version to a side file | `dvs get data.csv --version <hash> --to data_v1.csv` | None: real path + old hash is an uploaded pair | True: "downloaded data.csv as of version X" | New flags on `get`; output is untracked |
| O3 | Track a copy without re-uploading | `dvs copy data.csv@<hash> data_v1.csv` | New upload row at the new path, no bytes | True, but the copy becomes a tracked dataset | Comparison scratch ends up versioned and in git |
| O4 | Do nothing; use a second git worktree | `git worktree add ../old <rev>` + `dvs get` there | None | True | Whole-checkout download; clunky for one file |

## D — Decisions

### D1 — Old versions come through `dvs get`

`dvs get <file> --version <v> --to <out>` writes version `v` of a tracked file to `out`. Without
`--version`, `get` behaves as today.

Flags on `get`, not a new command. On the server it is still a download, audited as "user X got F
at version V"; it only writes the bytes somewhere else.

*Decided — Wes, 2026-11-10.*

### D2 — A version is named by a hash from `dvs history`

`--version` takes a BLAKE3 hash or a unique prefix (8+ characters) of one, as `dvs history <file>` prints it.

Hash prefix only. The usual case is a known QC'd hash compared against the current file, so the
hash is in hand, not guessed from a commit or date.

*Decided — Wes, 2026-11-10.*

### D3 — The request names the real tracked path

The client asks the server for `data.csv` at the old hash, which is an uploaded pair, so no server
change is needed and the download is audited against the right file.

*Decided — my call, reversible.*

### D4 — A hash the file never had is refused

If `data.csv` was never stored at that hash, `get` fails with "data.csv has no version <v>", on
both backends. The local backend checks its audit log, so a hash belonging to another file is not
served there either.

*Decided — Wes, 2026-11-10: same rule on both backends.*

### D5 — The side file is untracked and git-ignored

`--to` writes a plain file: no sidecar, no status row, and `dvs status` does not list it. When it
lands inside the git repo, dvs adds it to `.gitignore` the same way `dvs add` ignores data files;
outside the repo nothing else is written.

*Decided — Wes, 2026-11-10 (auto-ignore).*

### D6 — `--to` is required and never overwrites

No default name. `get` refuses when `out` exists, is a tracked path, or is inside the metadata folder.

*Decided — Wes, 2026-11-10: no default name for now.*

### D7 — One file per call

`--version` takes exactly one file path; no folders, globs or `--recursive`.

*Decided — my call, reversible.*

### D8 — The R package gets the same option

`dvs_get()` in dvs-rpkg gains `version` and `to` arguments that call the same code.

*Decided — my call, reversible.*

### D9 — (withdrawn) No git commit on downloads

Recording the git branch/commit on downloads was considered and dropped: downloads do not record
it today and this feature does not need it.

*Decided — Wes, 2026-11-10.*

### D10 — Built from main, independent of migration

The work branches off `main`, not `feat/migration`, and nothing in it depends on dvs-migrate.

*Decided — Wes, 2026-11-10.*

## W — Walks

### W1 — Compare the current dataset with an older one

1. `dvs history data.csv` lists past versions: hash, who, when, message.
2. The user picks one: `dvs get data.csv --version 1a2b3c4d --to compare/data_v1.csv`.
3. dvs resolves the prefix against that file's history, then downloads `data.csv` at that hash.
4. The server finds the upload of (data.csv, that hash) and records the download against it.
5. The bytes land in `compare/data_v1.csv`. `data.csv` and its sidecar are untouched.
6. The user compares the two files, then deletes the side file when done.

## I — Invariants

### I1 — The server serves a hash only at a path it was uploaded at

Unchanged from today; O2 needs no exception to it.

### I2 — Fetching an old version never changes tracked state

No sidecar, working copy or local-state record changes. The only write besides the side file is
its `.gitignore` entry (D5).

### I3 — Every old-version download is audited under the real path

The record names the user, the tracked path and the version. No git commit is recorded for
downloads; none is today either.

## Q — Resolved whole-design questions

### Q1 — Is the need a throwaway comparison or a pinned second dataset?

If the user commits `data_v1.csv.dvs` so collaborators and scripts can rely on it, the side file in
O2 is not enough and O3 (a tracked copy without re-upload) fits better.

*Decided — Wes, 2026-11-10: throwaway for now → O2.*

## Appendix — Implementation phases (`P`)

Branch off `main` (D10). No dvs-server change (D3, I1).

- **P1 — Version resolution (dvs crate).** Resolve a hash or 8+ char prefix against the file's
  history via `Backend::get_audit_entries`; unknown or ambiguous prefix → coded error (D2, D4).
  Same path on local and server. Files: `dvs/src/files/get.rs` (or a new `dvs/src/files/version.rs`),
  `dvs/src/backends/mod.rs` if a helper is needed.
- **P2 — Fetch to side file (dvs crate).** Validate `--to` (D6), retrieve with the real tracked
  path + resolved hash (D3), write to `out`, add `out` to `.gitignore` when inside the repo via
  `gitignore::add_to_gitignore` (D5). No sidecar or local-state writes (I2). Files:
  `dvs/src/files/get.rs`, `dvs/src/gitignore.rs` (visibility only, if needed).
- **P3 — CLI flags (depends on P1, P2).** `--version` / `--to` on `Get`, requiring each other and
  conflicting with `--glob`, `--recursive`, multiple paths (D1, D7). Files: `dvs-cli/src/main.rs`,
  `dvs-cli/src/get_cmd.rs`.
- **P4 — R package (depends on P1, P2).** `version` / `to` on `dvs_get()` (D8). Files:
  `dvs-rpkg/R/dvs-commands.R`, `dvs-rpkg/src/` bindings.
