# Batch Outcome Type — Partial-Success Results Across Core, R, and CLI

**Status:** Draft
**Date:** 2026-04-23
**Scope:** `dvs` core crate, `dvs-rpkg` R bindings, `dvs-cli`
**Commands affected:** `add`, `get`, `status`

## Motivation

`dvs add`, `dvs get`, and `dvs status` operate on batches of paths, run in parallel via rayon, and can partially succeed. The current API mixes per-item successes and errors into a single flat `Vec<AddResult>` / `Vec<GetResult>` / `Vec<FileStatus>`, where each item carries a `detail: Success | Error { error: String }` variant. Top-level failures (glob resolution, config discovery) still short-circuit the batch via `anyhow::bail!`.

Two concerns with the current shape drove this design:

- **Errors are unstructured.** Free-form strings force downstream consumers (CLI grouping, R tabular output, tests) to regex the message. Devin's Slack thread makes the requirement explicit: "we must report all errors when we return" — and the user must be able to reason about them, not string-match.
- **Pre-flight vs runtime errors are conflated.** Glob resolution bails before the batch runs, losing the rest of the input. This contradicts the guiding principle: *check what we can before kicking off; report everything at the end.*

The goal of this spec is to introduce a single shape — `BatchOutcome<S, E>` — that cleanly separates successes from failures, carries structured per-command error enums, and surfaces consistently through the R bindings (`list(ok = …, err = …)`) and CLI output.

## Execution model

**One mode.** Every batch operation always:

1. Validates every input (pre-flight). Failures become error rows.
2. Dispatches every remaining input to rayon. Failures become error rows.
3. Returns *all* ok rows and *all* err rows at the end.

There is no fail-fast flag, no stop-scheduling knob, no cancellation token. A batch invocation is a single atomic "do what you can" pass; the caller gets the complete ledger back and decides what to do. The CLI exits non-zero when the ledger contains any err rows.

## Non-goals

- Transactional rollback of partial writes.
- Retry policy on transient errors.
- Mid-flight cancellation or stop-scheduling behavior (explicitly excluded; see "Execution model" above).
- Changes to progress reporting.
- Changes to per-file concurrency tuning.

These are deliberately out of scope; they belong to follow-up designs.

## Design

### 1. Core type

New module `dvs/src/batch.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Result of a batch operation that may partially succeed.
///
/// `S` = success row, `E` = error row. Each element carries its own
/// `path` (or identifying field) inside `S` / `E` — the outcome itself
/// is intentionally path-agnostic so it can be reused across commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutcome<S, E> {
    pub ok:  Vec<S>,
    pub err: Vec<E>,
}

impl<S, E> BatchOutcome<S, E> {
    pub fn is_clean(&self) -> bool { self.err.is_empty() }
    pub fn split(self) -> (Vec<S>, Vec<E>) { (self.ok, self.err) }
}
```

`BatchOutcome` replaces `Vec<AddResult>` / `Vec<GetResult>` / `Vec<FileStatus>` at the public API boundary of `dvs`. The current `*Result { path, detail: Success|Error }` types are deleted in favor of separate success and error types (see §2).

### 2. Per-command success and error enums

Each operation gets its own concrete success struct and error enum. No generic payloads — explicit variants per failure mode. All error enums use `#[serde(tag = "kind", rename_all = "snake_case")]` so the tag becomes a column in R and JSON output.

#### Add

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,        // Copied | Present
    pub hash: String,
    pub size: u64,
    pub stored_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddError {
    /// Pre-flight: the path does not exist on disk.
    NotFound       { path: PathBuf },
    /// Pre-flight: path escapes the repo root (e.g., `../elsewhere`).
    OutsideProject { path: PathBuf },
    /// Pre-flight: path refers to a directory; `dvs` versions files.
    IsDirectory    { path: PathBuf },
    /// Pre-flight: path could not be canonicalized.
    PathResolution { path: PathBuf, reason: String },
    /// Pre-flight: glob pattern could not be compiled or matched.
    GlobFailure    { pattern: String, reason: String },
    /// Runtime: hashing the file failed.
    HashFailure    { path: PathBuf, reason: String },
    /// Runtime: writing to the backend failed.
    StorageWrite   { path: PathBuf, reason: String },
}
```

#### Get

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GetError {
    NotFound        { path: PathBuf },
    NotTracked      { path: PathBuf },
    GlobFailure     { pattern: String, reason: String },
    MetadataRead    { path: PathBuf, reason: String },
    StorageMissing  { path: PathBuf, hash: String },
    StorageRead     { path: PathBuf, reason: String },
    HashMismatch    { path: PathBuf, expected: String, got: String },
}
```

#### Status

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSuccess {
    pub path: PathBuf,
    pub status: Status,                         // Current | Absent | Unsynced | Untracked
    pub metadata: Option<FileMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StatusError {
    RelativePath  { metadata_path: PathBuf, reason: String },
    MetadataRead  { path: PathBuf, reason: String },
    HashFailure   { path: PathBuf, reason: String },
}
```

### 3. Public API signatures

```rust
pub fn add_files(
    paths: Vec<PathBuf>,
    dvs_paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<BatchOutcome<AddSuccess, AddError>>;

pub fn get_files(
    paths: Vec<PathBuf>,
    dvs_paths: &DvsPaths,
    backend: &dyn Backend,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<BatchOutcome<GetSuccess, GetError>>;

pub fn get_status(
    dvs_paths: &DvsPaths,
    filter: Option<&StatusFilter>,
) -> Result<BatchOutcome<StatusSuccess, StatusError>>;
```

The outer `Result<_, anyhow::Error>` is reserved for **batch-scope fatal** errors — situations where no meaningful partial result can be produced:

- No DVS repo discoverable from `cwd`.
- `.dvs` metadata folder unreadable.
- Thread pool initialization failed.
- Programmer error (unwrap of an invariant).

Every per-path or per-pattern problem is a row in `err`, not an outer `Err`.

#### Glob resolution moves inside the function

Today `resolve_paths_for_add` / `resolve_paths_for_get` return `anyhow::Result<Vec<PathBuf>>` and short-circuit the first bad pattern. They change to:

```rust
pub fn resolve_paths_for_add(
    inputs: Vec<PathBuf>,
    glob: Option<&str>,
    dvs_paths: &DvsPaths,
) -> (Vec<PathBuf>, Vec<AddError>);
```

Failing patterns become `AddError::GlobFailure { pattern, reason }` rows; resolvable paths continue to the parallel pass. Same shape for `get`.

### 4. Parallel execution

Existing `par_iter().map().collect()` pattern, unchanged semantics. All scheduled tasks run to completion; results (ok or err) are sorted by path and returned. Pre-flight error rows and parallel-phase error rows are concatenated into a single `err` vec, then sorted by path alongside their successful siblings.

No shared cancellation state. No early exit. One way through.

### 5. R bindings

`dvs-rpkg/src/rust/lib.rs` adds a conversion helper:

```rust
fn outcome_to_r<S, E>(b: BatchOutcome<S, E>) -> List
where S: Serialize, E: Serialize,
{
    let mut l = miniextendr_api::list!();
    // $ok is always present (zero rows if nothing succeeded) so R callers
    // can index it unconditionally.
    l.set("ok", vec_to_dataframe(&b.ok)?);
    // $err is present only when there are failures — matches the user's
    // stated convention: "only one side if only one is present".
    if !b.err.is_empty() {
        l.set("err", vec_to_dataframe(&b.err)?);
    }
    l
}
```

`dvs_add_impl`, `dvs_get_impl`, `dvs_status_impl` return `List` instead of `DataFrame`. The user-facing R wrappers in `dvs-commands.R` surface the list directly — callers access `$ok` and `$err`, both columnar data frames. The `err` frame carries a `kind` column populated from the serde tag.

**R-side ergonomics:**

```r
res <- dvs::dvs_add(c("a.csv", "b.csv", "bad/path.csv"))
res$ok   # data frame of successes (may have 0 rows)
res$err  # data frame of failures, present only if any; has `kind`, `path`, and kind-specific columns
```

### 6. CLI presentation

After each `add` / `get` / `status` op, output is **errors first, then successes** (the user's explicit convention — failures are what the user needs to act on, they go at the top where they won't scroll off):

```
Failed (4):
  not_found (2):
    foo/missing.csv
    bar/missing.csv
  outside_project (1):
    ../elsewhere/x.txt
  is_directory (1):
    data/

Added 12 files (3.4 MB stored, 12.1 MB source)
```

If `err` is empty, the failure block is omitted entirely. If `ok` is empty, the success line is omitted (or replaced with `No files processed.`).

Grouping key: the serde `kind` tag. Count per kind is rendered in the section header. Within a group, paths are sorted.

**Exit codes:**

| State | Exit |
|---|---|
| `err.is_empty()` | `0` |
| any per-item err | `1` |
| batch-scope fatal (outer `anyhow::Err`) | `2` |

### 7. Testing strategy

#### Core crate

- Unit test per error variant: constructible, serializes to the expected JSON (`kind: "not_found"`, etc.).
- `BatchOutcome::is_clean`, `split` semantics.
- Existing `add_files_mixed_statuses` / `get_files_reports_not_*` tests migrate to assert on typed enum variants, not `error.contains(...)`.
- New integration tests:
  - `add_files` with mixed inputs: valid path, nonexistent path, outside-project path, and a directory all appear in the outcome — successes in `ok`, failures in `err` with correct `AddError` variants.
  - Malformed glob + resolvable paths: the bad pattern becomes `GlobFailure`, the good paths still run.

#### R package

- `testthat::test_that("dvs_add returns only $ok when nothing fails", …)`.
- `test_that("dvs_add returns $ok and $err with kind column when mixed", …)`.
- `test_that("dvs_add returns $ok (zero rows) and $err when all fail", …)`.
- `test_that("empty repo returns $ok with zero rows, no $err", …)`.

#### CLI

- Snapshot test on the failure-then-success formatter for 3–5 representative mixed-outcome batches.
- Exit-code test: mixed outcome produces `1`, clean produces `0`, fatal (no repo) produces `2`.

### 8. Migration path

Breaking change to the `dvs` public API surface. Sequenced:

1. Introduce `BatchOutcome`, error enums, and success structs in `dvs` without deleting the old types; new API alongside old.
2. Port `add_files`/`get_files`/`get_status` to return `BatchOutcome`.
3. Update `dvs-cli` and `dvs-rpkg` callers (including the error-first output ordering in CLI).
4. Delete the old `AddResult`/`GetResult`/`FileStatus` + `*Detail` enums; remove the now-unused `Result<Vec<_>>` shape.

Steps 1–3 are one PR per crate boundary; step 4 is a cleanup PR.

## Future work

- **Retry policy.** A `RetryPolicy` enum for transient backend errors on individual ok-path attempts.
- **Structured error codes in JSON output.** The `kind` tag is the seed; a stable error-code registry (`DVS-ADD-NOT-FOUND`, etc.) can layer on top for scripting.
- **Rollback.** Today a partial `add` leaves partial state in the backend. A transactional mode is a separate design.
