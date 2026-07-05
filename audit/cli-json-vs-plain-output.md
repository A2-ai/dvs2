# CLI output: --json vs plain

Based on `origin/main` at `b3908f6` (dvs-cli 0.3.0), macOS arm64, captured
2026-06-24. Every command was run twice in a fresh repo, once plain and once
with `--json`, with stdout, stderr, and exit code captured separately.

## The rule

`--json` only governs the result set. If a command produces per-file results,
those results (including per-file failures) are emitted as JSON on stdout. If it
rejects the input before that point, or clap rejects the arguments, the output
is plain text on stderr with an empty stdout and the `--json` flag is ignored.
The `Error: Some files failed to ...` summary always goes to stderr, even in
JSON mode.

Three failure classes:

| Class | When | stdout | stderr | --json effect | exit |
|-------|------|--------|--------|---------------|------|
| Pre-flight | input rejected (missing, untracked, outside project, no glob match, init guards) | empty | `Error: ...` | none, never emits JSON | 1 |
| Argument | clap rejects (required arg missing) | empty | usage text | only adds `--json` to the usage line | 2 |
| Post-start | batch passed validation, then a file fails (permission, missing or corrupt blob, unparseable sidecar) | results | per-file error plus summary | failed file becomes a JSON `{"path","error"}` entry on stdout | 1 |

Exit codes are the CLI process exit. `2` is always a clap argument error. `1`
covers both "input rejected, nothing happened" and "batch started, then a file
failed". Only the stdout and stderr shape tells those two apart.

## Success output, per command

JSON always prints to stdout, exit 0. The top-level type differs. `init`
returns an object. `add`, `get`, and `status` return an array with one entry per
file.

### init

```
plain   stdout:  DVS Initialized at "/.../repo"
--json  stdout:  {"status":"initialized"}
```

`init` is the only one whose JSON is a single object, not an array. It carries
no path or storage detail, only the status string.

### add

```
plain   stdout:  Added: data/a.bin [1.0 KB] --> saved [1.0 KB] as 982cc726...
--json  stdout:  [{"path":"data/a.bin","outcome":"copied","hash":"0ad32d3b...","size":1024,"stored_size":1033}]
```

`outcome` reflects dedup. A new blob is `"copied"`. Re-adding identical content
already in storage is `"present"`, with `stored_size == size` and nothing
re-written.

### add --dry-run

```
plain   stdout:  To add: data/a.bin [1.0 KB] as b46a65aa...
--json  stdout:  [{"path":"data/a.bin","outcome":"copied","hash":"3af6a1b0...","size":1024,"stored_size":1024}]
```

The plain text says "To add", but the `--json` dry-run output has the same shape
as a real add. `outcome` is still `"copied"` and there is no dry-run marker. A
JSON consumer cannot tell a dry run from a real run. Track that flag separately.

### get

```
plain   stdout:  data/a.bin [1.0 KB]
                 Total: 1 files, 1.0 KB
--json  stdout:  [{"path":"data/a.bin","outcome":"copied","size":1024}]
```

`get` JSON is leaner than `add` JSON. It has `path`, `outcome`, `size`, and
omits the `hash` and `stored_size` fields that `add` includes. `get --dry-run`
JSON is identical to a real get.

### status

```
plain   stdout:  +------------+---------+--------+
                 | path       | status  | size   |
                 +------------+---------+--------+
                 | data/a.bin | current | 1.0 KB |
                 +------------+---------+--------+
--json  stdout:  [{"path":"data/a.bin","status":"current","metadata":{
                   "hashes":{"blake3":"eb693d5f..."},"size":1024,
                   "created_by":"elea","add_time":"2026-06-24T13:33:49.97Z",
                   "compression":"zstd"}}]
```

### status --with-metadata

The plain table widens from 3 columns to 8 (adds hash, created_by, add_time,
compression, message). The JSON is byte-identical to `status --json`. The
`--with-metadata` flag affects only the plain table. The JSON always carries the
full `metadata` object regardless of the flag.

## Error output, per command

In each table, plain and --json were the same command run twice. "same as plain"
means the `--json` flag produced no JSON. The error came back as plain text on
stderr with an empty stdout.

### add

| Invalid input | exit | plain (stderr) | --json |
|---------------|------|----------------|--------|
| `add data/nope.bin` (single missing) | 1 | `Error: Path not found: data/nope.bin` | same as plain, no JSON |
| `add data/a.bin data/nope.bin` (valid + missing, a.bin NOT added) | 1 | `Error: Path not found: data/nope.bin` | same as plain, no JSON |
| `add data/sub` (bare directory) | 1 | `Error: No files to add` | same as plain, no JSON |
| `add --glob '*.zzz'` (no match) | 1 | `Error: No files to add` | same as plain, no JSON |
| `add /etc/hosts` (outside project) | 1 | `Error: Refusing to add, the following paths are invalid:` then `/etc/hosts: path is outside project` | same as plain, no JSON |
| `add` (no paths, no glob) | 2 | clap usage, `Usage: dvs add <PATHS>...` | same usage, line reads `dvs add --json <PATHS>...` |
| `add data/ok.bin data/bad.bin` (bad.bin is chmod 000, post-start) | 1 | stdout `Added: data/ok.bin ...`, stderr `Error adding data/bad.bin: failed to open ... Permission denied (os error 13)` then `Error: Some files failed to add` | stdout `[{"path":"data/bad.bin","error":"failed to open ... Permission denied"},{"path":"data/ok.bin","outcome":...,"hash":...}]`, stderr keeps `Error: Some files failed to add` |

### get

| Invalid input | exit | plain (stderr) | --json |
|---------------|------|----------------|--------|
| `get data/nope.bin` (single untracked) | 1 | `Error: The following paths are not tracked by DVS:` then `data/nope.bin` | same as plain, no JSON |
| `get data/a.bin data/nope.bin` (tracked + untracked, a.bin NOT retrieved) | 1 | `Error: The following paths are not tracked by DVS:` then `data/nope.bin` | same as plain, no JSON |
| `get --glob '*.zzz'` (no match) | 1 | `Error: No files to get` | same as plain, no JSON |
| `get` (no paths, no glob) | 2 | clap usage, `Usage: dvs get <PATHS>...` | same usage, `dvs get --json <PATHS>...` |
| `get data/a.bin` (storage blob deleted, post-start) | 1 | stderr `Error: data/a.bin - Storage file missing for hash: Hashes(blake3=55ce47a8...)` then `Error: Some files failed to get` | stdout `[{"path":"data/a.bin","error":"Storage file missing for hash: Hashes(blake3=55ce47a8...)"}]`, stderr keeps `Error: Some files failed to get` |

### status

`status` never rejects input. An untracked path argument is dropped silently and
exits 0. It only exits 1 when a tracked file's metadata cannot be read, a
post-start error, and that file still appears as an `error` entry next to the
healthy ones.

| Input | exit | plain | --json |
|-------|------|-------|--------|
| `status data/nope.bin` (untracked path argument) | 0 | stdout `No tracked files` | stdout `[]` |
| `status` with one corrupted sidecar (post-start) | 1 | stdout the table of healthy rows, stderr `Error getting status for data/a.bin: expected ident at line 1 column 2` then `Error: Some files failed to get status` | stdout `[{"path":"data/a.bin","error":"expected ident..."},{"path":"data/b.bin","status":"current","metadata":{...}}]`, stderr keeps the summary |

### init

`init` validation guards never emit JSON. Plain and --json are identical.

| Invalid input | exit | stderr (plain and --json) |
|---------------|------|---------------------------|
| storage path inside the repo | 1 | `Error: The given storage path is within the repository.` |
| re-init an initialized repo | 1 | `Error: dvs is already initialized (dvs.toml exists)` |
| `init --group nonexistent` | 1 | `Error: Group 'nonexistent_grp_xyz' not found` |
| `init` (missing PATH arg) | 2 | clap usage, `Usage: dvs init <PATH>` |

## Notes for JSON consumers

1. Check the exit code first, parse stdout second. A nonzero exit with an empty
   stdout means a pre-flight or argument rejection. The reason is plain text on
   stderr, never JSON. Parse stdout only when it is non-empty.
2. Exit codes do not distinguish rejection from partial failure. `2` is always a
   clap argument error. `1` covers both "input rejected, nothing happened" and
   "batch started, then a file failed". Only the stdout and stderr shape tells
   them apart.
3. The batch-failure summary is on stderr even in JSON mode. A consumer reading
   only stdout sees the per-file `error` entries but not the `Error: Some files
   failed to ...` line. Infer overall failure from the nonzero exit.
4. `add`, `get`, and `status` do not share a schema. `add` entries carry `hash`
   and `stored_size`. `get` entries do not. `status` entries use `status` and a
   nested `metadata` object. `init` returns a bare object. One parser does not
   fit all four.
5. `--dry-run` is invisible in JSON. Dry-run JSON is identical to a real run for
   both `add` and `get`. Track the flag on the caller side.
6. Bad input paths refuse the whole batch (#240 fail-fast) before any work, so a
   bad input path never produces a partial JSON array. Partial arrays (mixed
   success and `error` entries) come only from failures during a valid batch.

Note: long hashes and paths in the error tables are truncated with `...` for
readability. The field shapes are literal.
