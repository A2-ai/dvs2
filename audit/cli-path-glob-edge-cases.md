# CLI behavior for wrong paths and empty globs

Based on `origin/main` at `dd60555`. The relevant code is identical on PR #231
(`docs/cli-edge-cases`), which is spec-only. Verified by building the CLI from
that branch and running an empirical matrix (`add`, `get`, `status` against
tracked, untracked, missing, partial, glob-hit, glob-miss, trailing-slash, and
outside-repo inputs), cross-checked against the resolution code
(`dvs/src/globbing.rs`, `dvs/src/paths.rs`, `dvs/src/files/status.rs`,
`dvs-cli/src/main.rs`).

This is the premise behind PR #231: `status` zero-match exits `0` while
`add`/`get` exit `1`, and a trailing slash on a file path is normalized away.
Both hold. The investigation also found the `status` behavior is broader than
"zero-match": any wrong path is dropped silently.

## The matrix

`status` never refuses an input. `get` and `add` are fail-fast all-or-nothing:
one bad path refuses the whole batch and nothing is written or retrieved.

| Condition | `status` | `get` | `add` |
|-----------|----------|-------|-------|
| all paths valid / tracked | `0`, rows shown | `0`, retrieves | `0`, adds |
| partial (1 good + 1 bad) | `0`, shows good, **drops bad silently** | `1`, "not tracked by DVS", retrieves nothing | `1`, "Path not found", adds nothing |
| fully wrong (nonexistent) | `0`, empty / "No tracked files" | `1`, "not tracked by DVS" | `1`, "Path not found" |
| untracked but on disk | `0`, dropped | `1`, "not tracked" (same as nonexistent) | n/a, this is the add target |
| glob hit | `0`, rows shown | `0`, retrieves | `0`, adds |
| glob miss | `0`, "No tracked files" | `1`, "No files to get" | `1`, "No files to add" |
| bare directory, no glob | `0`, filters tracked under it | `0`, tracked under it | `1`, "No files to add" |
| trailing slash on a file (`x.bin/`) | `0`, matches `x.bin` | `0`, retrieves `x.bin` | `0`, adds `x.bin` |
| path outside repo (`../x`) | `0`, escapes root, dropped | `1`, not tracked | `1`, "Path not found" |

Exit codes are the CLI process exit. In R the same conditions raise an error
where the CLI exits `1` on resolution, and surface per-file runtime failures as
an `error` column instead.

## Why each command behaves this way

### `status` is a predicate over the tracked set, never a validator

`get_status` (`dvs/src/files/status.rs:60`) iterates only the already-tracked
files (`tracked_paths`, `dvs/src/paths.rs:165`) and keeps each one that the
user's paths or glob match (`PathFilter::matches`, `dvs/src/paths.rs:296`). A
named path is never checked against the disk or the metadata. It is only used
to filter. A path that matches nothing simply contributes no row.

The single way `status` exits `1` is a tracked file whose own `.dvs` metadata
fails to parse, which becomes a `StatusDetail::Error` row
(`dvs/src/files/status.rs:106`) and trips `has_errors`
(`dvs-cli/src/main.rs:401`). Wrong, missing, untracked, partial, and
outside-repo inputs all exit `0`.

### `get` validates every explicit path up front and refuses the batch

`resolve_paths_for_get` (`dvs/src/globbing.rs:95`) requires every explicit path
to match at least one tracked file. Any path that matches nothing is collected
and the whole call bails (`dvs/src/globbing.rs:130`, "Bail on any invalid path",
#240). A no-paths glob can legitimately resolve to empty, which the CLI turns
into "No files to get" (`dvs-cli/src/main.rs:418`). An untracked-but-on-disk
path is indistinguishable from a nonexistent one: both report "not tracked".

### `add` canonicalizes each explicit path, and a bare directory needs a glob

`resolve_paths_for_add` (`dvs/src/globbing.rs:27`) calls `canonicalize()` on
each explicit path. A missing path or one that escapes the repo becomes "Path
not found: ..." (`dvs/src/globbing.rs:49`). A directory contributes files only
through a glob walk (`dvs/src/globbing.rs:59`), so a bare directory with no glob
resolves to nothing, which the CLI turns into "No files to add"
(`dvs-cli/src/main.rs:243`). A second gate, `validate_for_add`
(`dvs/src/paths.rs:204`, called from `dvs/src/files/add.rs:99`), backstops
is-directory and outside-project. After a valid batch starts, a per-file copy
failure marks that file failed, adds the rest, and still exits `1`.

## Trailing slash is normalized, via two different code paths

- `add`: `canonicalize()` collapses `x.bin/` to `x.bin`
  (`dvs/src/globbing.rs:48`). On macOS this succeeds even though POSIX
  `realpath` would return `ENOTDIR` for a trailing slash on a regular file.
  Verified: the file is copied and the command exits `0`.
- `get` and `status`: `normalize_path` (`dvs/src/paths.rs:88`) walks
  `components()`, and a trailing separator produces no extra component, so it is
  dropped. Verified: `get 'data/raw/a.bin/'` actually copies the file rather
  than no-op.

## A glob can "miss" because of the separator, not because files are absent

`--glob` uses a literal path separator. `add data/ --glob '*.bin'` resolves to
"No files to add" even when `data/raw/*.bin` exist, because `*.bin` matches only
direct children of `data/`. Reaching nested files needs `--glob '**/*.bin'`.
This is documented in the Globbing section of `specs.md`, but it is the most
common reason a glob looks like it "matched nothing".

## The gap PR #231 leaves, and the fix

PR #231 documents the `status` zero-match case ("a path set or glob that matches
no tracked files is not an error"). It does not call out the partial case:
`dvs status good.bin bogus.bin` exits `0`, shows only `good.bin`, and drops
`bogus.bin` with no diagnostic. That is the sharper asymmetry against `get`,
which refuses the whole batch. PR #250 adds this to the `status` section,
stating that each named path filters independently and a non-matching one is
dropped silently.

## Related

- PR #231 (`docs/cli-edge-cases`): the status zero-match and trailing-slash
  spec additions.
- PR #250 (`docs/status-partial-path-drop`): the partial-path sharpening this
  audit motivated. Self-contained off `main`, merges cleanly with #231 in
  either order (verified with `git merge-tree`).
- `get-semantics-overview.md`: the `get` resolution model in detail.
- The fail-fast all-or-nothing behavior is #240.
