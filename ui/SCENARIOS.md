# dvs CLI scenario matrix

Exhaustive scenario coverage for the `dvs` CLI. Every spec claim plus every edge
case we can think of: existing/missing files, overwriting, absolute vs relative
vs mixed paths, `.`/`..`, trailing slashes, duplicates, symlinks, nested and
multiple projects, state transitions, compression, parallelism.

Each scenario is exercised by a `cases_*.sh` script that prints, per scenario,
`expected` vs `actual` and a `PASS`/`FAIL` verdict, ending with
`=== SUMMARY: N pass, M fail ===`. Scripts assert the SPEC-CORRECT behavior.
Where the current CLI is known to diverge (the filed issues), the case is
labeled with the issue number so a FAIL is recognizable as a tracked bug rather
than a test defect.

Conventions (mirror `validate_*.sh` / `main_status.sh`):
- `set -eu`, ERR trap, `source helpers.sh`, `set -xo pipefail`.
- `check WANT GOT LABEL` verdict helper; `say` for headers.
- Pristine repo + sibling storage per scenario group via
  `mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX`; storage is a sibling under `ui/`
  (NEVER inside the repo — init rejects storage within the project).
- `mkfiles N SIZE DIR` / `mkrandfile PATH SIZE` for fixtures. `git init -q` when
  gitignore behavior matters.
- Use `python3` for JSON (always present); avoid `jq` unless guarded.
- Capture exit codes: `rc=0; dvs ... || rc=$?`.
- Extract expected `--help` text from `../specs.md` rather than hardcoding.
- CLI only. The `dvs` binary is already installed from this worktree — do not reinstall.

Known tracked divergences (label these in the scripts; do not let them masquerade
as test bugs): #216 same-invocation identical-content dedup race, #217 `get <dir> --glob`,
#218 `status --glob`/`-g`, #219 add best-effort on missing path, #220 get exit code on unknown path.

Legend: [ ] todo  [x] implemented  [~] hard/partial

---

## cases_paths.sh — path handling across add / get / status

- [ ] absolute path to an in-project file (add, then get, then status)
- [ ] relative path (`data/x.bin`)
- [ ] `./`-prefixed relative path (`./data/x.bin`)
- [ ] mix of absolute + relative paths in ONE invocation
- [ ] `..` traversal that stays inside the project (`data/../data/x.bin`)
- [ ] `..` traversal that escapes the project root (rejected)
- [ ] absolute path pointing OUTSIDE the project root (rejected)
- [ ] trailing slash on a FILE path (`data/x.bin/`) — error or normalized?
- [ ] the same path listed twice in one invocation (dedup of args, not a race)
- [ ] path to a file that does not exist (exit 1, reported)
- [ ] path that is the project root `.` itself
- [ ] deeply nested path (sidecar mirrors full depth)
- [ ] filename with spaces
- [ ] filename with unicode characters
- [ ] path reported in output is project-root-relative regardless of how it was given
- [ ] add via abs path then `get` via rel path resolves the same sidecar (and vice versa)

## cases_init.sh — init scenarios

- [ ] init in an empty dir (happy path; dvs.toml + .dvs created)
- [ ] init when dvs.toml ALREADY exists in target dir (error, exit 1, existing toml untouched)
- [ ] init in a SUBDIRECTORY of an existing dvs project (succeeds — local check, L71)
- [ ] init with relative storage path
- [ ] init with absolute storage path
- [ ] init with storage path INSIDE the repo (rejected: "within the repository")
- [ ] init with storage path that already exists and is empty (ok)
- [ ] init with storage path that already exists and is NON-empty (behavior? document)
- [ ] init --root-dir <existing dir> (toml lands there, not cwd)
- [ ] init --root-dir <nonexistent dir> (error or created? document)
- [ ] init --metadata-folder-name custom (.mymeta created, .dvs not)
- [ ] init --metadata-folder-name colliding with an existing dir
- [ ] init --no-compression (toml records none)
- [ ] init --group <user's own group> (applied + recorded)
- [ ] init --group <nonexistent group> (error, document)
- [ ] init --threads 0 (auto) and --threads N
- [ ] init --json (valid JSON, status initialized)
- [ ] init with PATH argument missing (clap usage error, exit 2)
- [ ] init partial-failure cleanup (unwritable storage -> toml + .dvs removed, retry works)
- [ ] init writes audit entry (action=init, settings, project_path)
- [ ] init then immediately add works (end-to-end after init)

## cases_add.sh — add scenarios

- [ ] add a single new file -> outcome copied
- [ ] add the same UNCHANGED file again -> outcome present (metadata not rewritten)
- [ ] modify the file locally, add again -> copied, new hash + size + add_time
- [ ] add a file that does not exist -> exit 1, reported
- [ ] add several files where one is missing -> best-effort: others added, missing reported, exit 1 (#219)
- [ ] add a bare directory (no glob) -> not added (error / no-op)
- [ ] add with -m "message" -> recorded in sidecar
- [ ] re-add unchanged file with a NEW -m -> message NOT updated (present, no rewrite)
- [ ] add an empty (0-byte) file
- [ ] add a file with spaces / unicode in the name
- [ ] add a read-only (chmod 0444) file -> succeeds
- [ ] add an unreadable (chmod 000) file -> reported failure, exit 1, siblings still added
- [ ] add two DIFFERENT-named identical-content files in SEPARATE invocations -> dedup, one blob
- [ ] add two identical-content files in ONE invocation -> #216 (race; label)
- [ ] add the same path twice in one invocation
- [ ] add a nested-path file -> sidecar mirrors path
- [ ] add --dry-run -> outcomes reported, NO sidecar, NO blob, NO gitignore change
- [ ] add --json -> array of {path, outcome, hash, size, ...}
- [ ] add results sorted alphabetically by path
- [ ] add updates the file's own-dir .gitignore with `/<name>`, no duplicate on re-add
- [ ] add in a repo with NO .git -> succeeds, no .gitignore written
- [ ] add writes audit entries (action=add, file path+hashes, compression)
- [ ] add a symlink to an in-project file -> resolved, sidecar at real target
- [ ] add a symlink whose target is OUTSIDE the project -> rejected
- [ ] add storage blob is content-addressed and read-only after write

## cases_get.sh — get scenarios

- [ ] get a tracked file after deleting the local copy -> copied, file restored, hash verified
- [ ] get a file that already matches metadata -> present (noop)
- [ ] get OVERWRITES a locally-MODIFIED (unsynced) file -> document: does get clobber local edits? (data-loss check)
- [ ] get a tracked file into a directory that does not exist locally -> dirs created
- [ ] get by absolute / relative / mixed paths
- [ ] get an unknown/untracked path -> #220 (silently dropped vs exit 1; label)
- [ ] get with NO local file AND no metadata -> exit 1 ("No files to get")
- [ ] get when the storage blob is missing/deleted -> error for that file, exit 1
- [ ] get with a tampered storage blob (hash mismatch) -> retrieved file deleted, fails, exit 1
- [ ] get --dry-run -> reports, writes nothing
- [ ] get --json -> array of {path, outcome, size}
- [ ] get -g and --glob both resolve via metadata folder
- [ ] get <dir> --glob (positional directory) -> #217 (label)
- [ ] get --glob '**/*' restores everything
- [ ] partial failure (1 good + 1 mismatch) -> good retrieved, exit 1

## cases_status.sh — status scenarios + state transitions

- [ ] status with a mix of current / absent / unsynced files (default shows all)
- [ ] --current / --absent / --unsynced single filters
- [ ] filter combinations (unions)
- [ ] path filter to a single file
- [ ] directory filter non-recursive (direct children only)
- [ ] -r/--recursive (all descendants)
- [ ] --with-metadata adds metadata columns
- [ ] --json shape and that it honors filters
- [ ] status from a NESTED subdirectory (project root discovery, paths root-relative)
- [ ] status with abs / rel path arguments
- [ ] status when NO files are tracked yet (empty result, exit 0)
- [ ] status on a path that has no metadata (untracked) -> document
- [ ] status with a malformed / unreadable sidecar -> per-file error, exit 1
- [ ] transition: add (current) -> modify (unsynced) -> add (current again)
- [ ] transition: add (current) -> delete local (absent) -> get (current again)

## cases_glob.sh — exhaustive globbing (add + get; status has none per #218)

- [ ] explicit files: glob ignored, exact files used
- [ ] explicit directory + glob: directory walked, filtered
- [ ] no paths + glob: walks cwd (add) / metadata (get) filtered
- [ ] literal separator: `'*.bin'` matches only target dir, NOT subdirs
- [ ] `'**/*.bin'` recursive across subdirs
- [ ] glob matching ZERO files -> error / empty, exit code
- [ ] glob run from a subdirectory (cwd-relative)
- [ ] glob with a directory baked into the pattern (`'data/*.bin'`)
- [ ] unquoted `*.bin` (shell-expanded) vs quoted `'*.bin'` (lib-expanded) — same result set
- [ ] glob plus explicit paths together
- [ ] status --glob / -g rejected -> #218 (label)
- [ ] get <dir> --glob positional -> #217 (label)

## cases_project.sh — project discovery & multiple projects

- [ ] run any command from a nested subdir -> walks up to find dvs.toml
- [ ] no dvs.toml anywhere up the tree -> clear error, exit 1
- [ ] nested project: init inside an existing project, nearest dvs.toml wins (blob lands in inner storage)
- [ ] two sibling projects in one git repo with different storage
- [ ] command run exactly at project root
- [ ] storage path shared by two projects (content-addressable dedup across projects)
- [ ] metadata folder custom name respected by add/get/status discovery

## cases_overwrite.sh — overwriting, state & config transitions

- [ ] add file A, overwrite A's CONTENT locally, status -> unsynced; add -> copied (new hash)
- [ ] add file A, replace A with identical content (same bytes) -> present (hash+size match)
- [ ] get over an unsynced local file -> overwrite local with stored version (document data-loss)
- [ ] add under zstd, flip dvs.toml to none, add a NEW file -> new file none, old file still zstd
- [ ] add under none, flip dvs.toml to zstd, get the OLD file -> still works (per-file compression, L356-357)
- [ ] re-add a file whose mtime changed but content identical -> present (cache mtime+size)
- [ ] add, corrupt the hash cache db, then status/add still succeed (cache optional)
- [ ] overwrite a stored read-only blob scenario is covered under get hash-mismatch

---

## Implementation status / findings

(filled in as scripts are written and run; record any NEW divergence here with a
proposed issue)

Validated against origin/main @ 8288f85 (installed `dvs-cli 0.3.0` rebuilt from this worktree).

| script | pass | fail | notes |
|--------|------|------|-------|
| cases_paths.sh | 44 | 0 | trailing-slash-on-file normalized (undoc); `..`-escape msg existence-dependent |
| cases_init.sh | 53 | 0 | relative storage path recorded verbatim; non-empty storage not guarded |
| cases_add.sh | 54 | 2 | FAILs are tracked bugs #216 (dedup race) + #219 (missing-path aborts siblings) |
| cases_get.sh | 40 | 0 | get-over-unsynced = silent data loss; #217 FIXED; #220 still divergent |
| cases_status.sh | 58 | 0 | status now globs `--glob`/`-g` (#218 fixed) but flag undocumented |
| cases_glob.sh | 36 | 0 | #217 + #218 FIXED; get globs cwd-relative vs status root-relative |
| cases_project.sh | 39 | 0 | get rejects abs paths + cwd-relative resolution; init refuses shared storage |
| cases_overwrite.sh | 23 | 0 | per-file compression + cache-optional hold; get clobbers unsynced |

### Findings (verified by hand against origin/main)

PRIOR ISSUES now FIXED on origin/main:
- #217 `get <dir> --glob` positional directory now walks+filters correctly.
- #218 `status --glob`/`-g` now accepted and globs the metadata folder. CLI matches the spec claim.

PRIOR ISSUES still present:
- #216 same-invocation identical-content dedup race (rename TOCTOU on shared `<hash>.tmp`).
- #219 a missing input path aborts the whole `add`; valid siblings not added (not best-effort).
- #220 an explicitly-listed unknown path in a mix is silently dropped, exit 0.

NEW findings (candidate issues):
- N1 (CLI bug): `dvs get` does NOT accept absolute paths and resolves positional paths relative to CWD, not project root — a path from `status`/`add` output can't be fed to `get` from a nested dir. `add` accepts abs; `get` does not.
- N2 (CLI inconsistency): from a subdir, `get --glob '*.bin'` resolves cwd-relative while `status --glob` resolves root-relative; spec says "the same way". (Same root cause as N1.)
- N3 (safety/spec gap): `dvs get` silently overwrites a locally-modified `unsynced` file — no warning/backup, exit 0. Spec silent.
- N4 (spec/help sync): `status` now has `-g, --glob <GLOB>` with an EMPTY help string, absent from the specs.md status block.
- N5 (spec tension): `dvs init` refuses to attach a second project to existing storage ("backend storage exists"), blocking shared-storage/cross-project dedup.
- minor: trailing-slash-on-file normalization; relative storage path recorded verbatim; status zero-match (empty, exit 0) vs add/get zero-match (error, exit 1) — all undocumented.

| script | pass | fail | new findings |
|--------|------|------|--------------|
