# CLI spec-validation TODO

Goal: validate **every CLI-observable claim** in `specs.md` (the `origin/main`
version, which this worktree is built from) through a `ui/` script. Each script
prints, per claim, an `expected` vs `actual` and a `PASS`/`FAIL` verdict, then a
final summary line `=== SUMMARY: N pass, M fail ===`.

The `dvs` CLI is already installed on PATH from this worktree (`just install-cli`).
**Do not reinstall.** R package is out of scope: validate the CLI only.

Tooling conventions (mirror existing `ui/main_status.sh`):
- `set -eu`, `trap 'printf "ERROR at %s:%d\n" ...' ERR`, then `source helpers.sh`,
  then `set -xo pipefail`.
- Use `say` for section headers (avoids xtrace noise).
- Create temp repos with `mktemp -d "$SCRIPT_DIR"/dvs_repo_cli_XXX`; derive
  `RUN_SUFFIX` and a sibling `dvs_storage_cli_$RUN_SUFFIX`. Cleanup via
  `bash ui/cleanup.sh` (trashes `dvs_repo_*`/`dvs_storage_*`/`dvs_fixture_*`).
- Prefer `mkfiles N SIZE DIR` (urandom) — no dataset download needed.
- A `dvs` repo needs a `.git` dir for gitignore claims: `git init -q` in the repo.
- Capture exit codes explicitly: `dvs add missing.csv; echo "EXIT=$?"` (note
  `set -e` — use `if ! dvs ...; then` or `rc=0; dvs ... || rc=$?`).
- Verdicts: helper like `check() { if [ "$1" = "$2" ]; then echo "PASS: $3"; else echo "FAIL: $3 (want=$1 got=$2)"; fi; }`.

Legend: [ ] todo  [~] partial / hard to test (note why)  [x] done

## STATUS: all 7 scripts written and run. Results below.

Per-script verdicts (against the installed `origin/main` CLI, dvs-cli 0.3.0):
- `validate_init.sh` — 23 pass, 0 fail
- `validate_add.sh` — all assigned claims pass; surfaced the dedup race (#216) + best-effort gap (#219)
- `validate_get.sh` — all assigned claims pass; surfaced exit-code wording gap (#220)
- `validate_status.sh` — 36 pass, 0 fail
- `validate_glob.sh` — surfaced status-glob spec bug (#218) + get-dir-glob CLI bug (#217)
- `validate_internals.sh` — 20 pass, 0 fail (re-confirmed dedup race #216)
- `validate_global.sh` — 14 pass, 0 fail, 1 untestable (DVS_NUM_THREADS count not observable via CLI)

All `[ ]` claims below are validated [x] except where marked [~] (hard/untestable) or where a
violation issue was filed.

---

## Script 1 — `validate_init.sh` (init)

- [ ] `init` creates `dvs.toml` in the current folder (specs L29, L80)
- [ ] `init` errors if `dvs.toml` already exists in target dir (L70)
- [ ] check is LOCAL: `dvs.toml` in a parent dir does NOT block nested init (L71)
- [ ] `--root-dir <DIR>` creates `dvs.toml` in that root instead of cwd (L92)
- [ ] `--metadata-folder-name <NAME>` uses custom metadata folder (default `.dvs`) (L94)
- [ ] `--no-compression` -> stored files recorded as `compression: "none"` (L98, verify via later add metadata)
- [ ] `--group <GRP>` sets unix group on storage dir/files (L96) [~] needs a group the user belongs to; use `id -gn`, else note skipped
- [ ] `--json` produces JSON output (L88)
- [ ] `init --help` text matches spec block L79-101 (usage line, args, option order: json, threads, root-dir, metadata-folder-name, group, no-compression)
- [ ] partial-failure best-effort cleanup: if storage creation fails after `dvs.toml` written, `dvs.toml` (and metadata folder if newly created) are cleaned so retry works (L73-74) [~] force by pointing storage at an unwritable path e.g. `/dev/null/x` or a read-only dir
- [ ] `init` writes an audit entry: action `init` with `settings` + `project_path` (L361-377) — inspect `<storage>/audit.log.jsonl`

## Script 2 — `validate_add.sh` (add)

- [ ] `add --help` text matches spec block L153-167 (option order: json, threads, glob, message, dry-run)
- [ ] `add` takes files; a bare directory arg does NOT work without a glob (L130) — `dvs add data/` should add nothing / error
- [ ] `-m/--message` recorded in the `.dvs` metadata `message` field (L130, L165)
- [ ] `add` has NO `--recursive` option: `dvs add --recursive ...` errors (L131)
- [ ] best-effort: mix of valid + missing file -> valid added, missing reported, still processes all (L133)
- [ ] metadata sidecar created at `<metadata>/<path>.dvs` mirroring file path (L136, L311)
- [ ] data file gitignored, `.dvs` sidecar NOT gitignored (L137)
- [ ] outcome `copied` for a new/changed file (L141)
- [ ] outcome `present` when hash+size match existing metadata; metadata NOT rewritten so `message` is not updated on re-add with new `-m` (L142-143)
- [ ] symlink resolved before adding (add a symlink to an in-project file) (L145)
- [ ] symlink whose target resolves OUTSIDE project root is rejected (L145)
- [ ] atomicity: no `.tmp` left in storage after add; no partial metadata (L147-148) [~] observe absence of leftovers
- [ ] `dvs add *.bin` (shell-expanded) works (L170)
- [ ] `--glob '*.bin'` (lib-expanded, quoted) works (L171)
- [ ] exit code `1` if one or more files could not be added (missing file / no perms) (L173)
- [ ] `--dry-run` reports outcomes but makes NO changes (no metadata, no storage blob) (L166)
- [ ] `--json` output for add (L162)
- [ ] results sorted alphabetically by path (L178)
- [ ] `add` writes audit entries: action `add` with `file` (path+hashes) + `compression` (L361-372)

## Script 3 — `validate_get.sh` (get)

- [ ] `get --help` text matches spec block L214-227 (option order: json, threads, glob (-g), dry-run)
- [ ] outcome `copied`: file retrieved from storage after deleting local copy (L202)
- [ ] outcome `present`: local already matches metadata -> noop (L203)
- [ ] hash verified after retrieval; on mismatch retrieved file deleted + fails for that file (L205-206) [~] force by tampering the stored blob or metadata hash
- [ ] `get` resolves paths/globs against the METADATA folder, not the working tree (L208) — delete local file then `get` by path still finds it
- [ ] `-g/--glob` short + long flag both work (L225)
- [ ] exit code `1` if one or more files could not be retrieved (L230)
- [ ] `--dry-run` reports outcomes but makes NO changes (L226)
- [ ] `--json` output for get (L223)

## Script 4 — `validate_status.sh` (status, CLI only)

- [ ] `status --help` matches spec block L260-276 (option order: json, threads, recursive, current, absent, unsynced, with-metadata)
- [ ] default (no flags) shows ALL tracked files regardless of state (L279)
- [ ] three states reported correctly: `current` (added+present), `absent` (metadata but no local), `unsynced` (local differs from metadata) (L47-49)
- [ ] `--current` filters to current only (L272, L279)
- [ ] `--absent` filters to absent only (L273)
- [ ] `--unsynced` filters to unsynced only (L274)
- [ ] multiple filters combine (union), e.g. `--current --absent` (L279-280)
- [ ] path filter to a single file (L266)
- [ ] path filter to a directory, non-recursive = direct children only (L271)
- [ ] `-r/--recursive` includes all descendants of a directory (L271)
- [ ] `--with-metadata` shows all metadata columns in table output (L275)
- [ ] exit code `1` if one or more files could not be inspected (L282) [~] make a `.dvs` sidecar unreadable / malformed
- [ ] `--json` output for status (L269)

## Script 5 — `validate_glob.sh` (globbing across add/status/get)

- [ ] explicit files: glob ignored, files added/retrieved directly (L433)
- [ ] explicit directory + glob: directory walked, filtered by glob (L434)
- [ ] no paths + glob: walks current directory filtered by glob (L435)
- [ ] literal separator: `'*.csv'` matches only target dir, NOT `subdir/file.csv` (L437-438)
- [ ] `'**/*.csv'` matches recursively across subdirs (L438)
- [ ] same glob rules hold for `add`, `status`, AND `get` (L431)

## Script 6 — `validate_internals.sh` (storage / metadata / compression / cache observable via CLI)

- [ ] metadata JSON fields present & well-formed: `hashes.blake3` (64 hex), `size` (bytes), `created_by`, `add_time` (ISO8601), `compression` (L317-334)
- [ ] `message` omitted from JSON when not provided (L334)
- [ ] storage layout content-addressable: blob at `<storage>/<2-char prefix>/<remaining 62>` matching the blake3 hash (L340-344)
- [ ] dedup: two files with identical content share one storage blob (L340)
- [ ] no `.tmp` files left in storage after add (L346-348)
- [ ] stored blobs are read-only (L350)
- [ ] compression default `zstd` recorded in metadata; `--no-compression` -> `none` (L352-355)
- [ ] changing `dvs.toml` compression AFTER an add does NOT break `get` of the earlier file (L356-357) — get reads compression from metadata
- [ ] hash cache exists at `{metadata}/.cache/dvs.db` after add (L405)
- [ ] `.cache` dir added to `.gitignore` (L407)
- [ ] cache is optional: delete/corrupt `dvs.db`, operations still succeed (L408)

## Script 7 — `validate_global.sh` (cross-cutting CLI surface)

- [ ] `dvs --help` lists subcommands in order: init, add, status, get, help (L27; see existing main_cli_help.sh)
- [ ] `--json` accepted by all four subcommands (L13)
- [ ] project root discovery: from a nested subdir, `dvs` walks up to find `dvs.toml` (L19-20) — run `dvs status` from `repo/data/raw/` and confirm it finds the project
- [ ] multiple projects in one git repo: nested `dvs.toml` resolves to the nearest one (L20, L71)
- [ ] gitignore: entry format `/<filename>` in the file's own dir `.gitignore`, no duplicate on re-add (L425-426)
- [ ] gitignore: skipped entirely when no `.git` folder, add still succeeds (L426-427)
- [ ] parallelism: `DVS_NUM_THREADS` honored (L412) [~] hard to observe count via CLI output; document approach or mark untestable

---

## Violations log

All reproduced by hand in a pristine repo before filing.

| claim | script | expected | actual | type | issue |
|-------|--------|----------|--------|------|-------|
| add atomicity / dedup (specs L340, L346-348) | add, internals | adding 2 identical-content files in one invocation succeeds (dedup to one blob) | under default parallel threads, `dvs add a b` (a==b) fails: rename TOCTOU on shared `<hash>.tmp`, exit 1, one sidecar missing. `--threads 1` works | CLI bug (high) | #216 |
| get glob = add resolution (specs L208, L434) | glob | `dvs get <dir> --glob '*'` walks dir like add | returns `No files to get`, exit 1, while `add <dir> --glob` and `get --glob 'dir/*'` both work | CLI bug | #217 |
| status accepts --glob (specs L431) | glob | `dvs status --glob` filters by glob | `status` has no glob flag; rejected exit 2 | spec wrong | #218 |
| add best-effort (specs L133) | add | failures don't stop other files | a missing/unresolvable path aborts the whole invocation; valid siblings not added | spec/CLI mismatch | #219 |
| get exit code (specs L230) | get | exit 1 if a requested file couldn't be retrieved | an explicit path with no metadata is silently dropped, exit 0 | spec under-specified | #220 |

Untestable via CLI: `DVS_NUM_THREADS` actual thread count (no thread-pool diagnostics in CLI output) — specs L412.
