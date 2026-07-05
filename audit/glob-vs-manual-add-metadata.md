# dvs add: `--glob` from root vs `cd` + individual add — metadata placement

**Date:** 2026-06-17
**Question:** Do these two ways of adding files land their `.dvs` metadata the same way?
- **Method A:** from repo root, one call: `dvs add data/derived-glob --glob '...'`
- **Method B:** `cd` into the data dir, add each file individually: `dvs add <file>`

**Answer:** Yes — identical, at any depth. Metadata placement keys off the
file's path **relative to repo root**, never the cwd or the glob-vs-explicit
choice.

## Why (code)

`resolve_paths_for_add` (dvs/src/globbing.rs:28-90) always resolves each file
to a repo-root-relative path before returning it:
- Explicit file (Method B): canonicalize `cwd().join(path)`, then
  `strip_prefix(repo_root)` (lines 53-59).
- Glob over a dir (Method A): walk entries, then `strip_prefix(repo_root)`
  (lines 76-80).

Metadata is then written at `.dvs/<repo-relative-path>.dvs`. So the two methods
cannot diverge by construction.

## Flat case

```
.dvs/data/derived-glob/file_N.bin.dvs      <- glob from root
.dvs/data/derived-manual/file_N.bin.dvs    <- cd + individual add
```

Same nesting, same `.dvs` extension, same JSON schema
(`hashes`/`size`/`created_by`/`add_time`/`compression`), same per-directory
`.gitignore`. Only the hashes (different content) and microsecond timestamps
differ.

## Nested case

Both trees mirror the full nested path identically:

```
.dvs/data/derived-{glob,manual}/
|-- file_top.bin.dvs
`-- sub
    |-- deep
    |   `-- file_low.bin.dvs
    `-- file_mid.bin.dvs
```

Two notes:
- **Recursive glob needs `**/`.** Method A used `--glob '**/*.bin'` to reach
  nested files. A plain `*.bin` matches only the top level, because the matcher
  uses `literal_separator(true)` (globbing.rs:18) so `*` stops at `/`. Method B
  is unaffected — you name each file.
- A `.gitignore` is written at **every** directory level holding tracked files
  (`sub/.gitignore`, `sub/deep/.gitignore`), each listing the local tracked
  files. Identical between the two trees.

## Reproduce

Probe script: `/tmp/dvs-glob-probe/probe.sh` (run `bash /tmp/dvs-glob-probe/probe.sh`).
Equivalent commands:

```bash
PROBE=/tmp/dvs-glob-probe; REPO="$PROBE/repo"; STORE="$PROBE/storage"
rm -rf "$REPO" "$STORE"; mkdir -p "$REPO" "$STORE"
cd "$REPO"; git init -q; dvs init "$STORE"

for base in data/derived-glob data/derived-manual; do
  mkdir -p "$base/sub/deep"
  head -c 1024 /dev/urandom > "$base/file_top.bin"
  head -c 1024 /dev/urandom > "$base/sub/file_mid.bin"
  head -c 1024 /dev/urandom > "$base/sub/deep/file_low.bin"
done

# Method A: one recursive glob from repo root
cd "$REPO"; dvs add data/derived-glob --glob '**/*.bin'

# Method B: cd to each level, add individually
cd "$REPO/data/derived-manual";         dvs add file_top.bin
cd "$REPO/data/derived-manual/sub";      dvs add file_mid.bin
cd "$REPO/data/derived-manual/sub/deep"; dvs add file_low.bin

cd "$REPO"; tree -a --noreport -I '.git' .dvs data
```
