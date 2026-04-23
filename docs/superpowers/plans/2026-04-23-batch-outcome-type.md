# BatchOutcome Partial-Success Type — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat `Vec<AddResult|GetResult|FileStatus>` APIs with a unified `BatchOutcome<S, E>` shape carrying structured per-command error enums, consumed by `dvs-cli` (errors-first grouped output) and `dvs-rpkg` (`list(ok, err)` helper).

**Architecture:** A single generic `BatchOutcome<S, E>` struct in `dvs/src/batch.rs` with per-command `{Add,Get,Status}{Success,Error}` types in `dvs/src/files/{add,get,status}.rs`. Glob resolution and pre-flight validation return structured error rows instead of `bail!`ing. Parallel execution stays on rayon unchanged; every scheduled task always runs. R bindings expose a `list(ok, err)` where `$ok` is always present and `$err` only when populated.

**Tech Stack:** Rust (workspace crates `dvs`, `dvs-cli`, `dvs-rpkg`), serde, rayon, anyhow, miniextendr, R/testthat, clap, tabled.

**Reference spec:** `docs/superpowers/specs/2026-04-23-batch-outcome-type-design.md`

**Branch:** `design/batch-outcome-type` (off `origin/main`).

**Global conventions used throughout the plan:**

- All `dvs` crate code is under `dvs/src/`, workspace root is `/Users/elea/Documents/GitHub/dvs2`.
- Run Rust tests with `cargo test -p dvs` or `cargo test -p dvs --lib <test_name>` (no workspace fan-out needed).
- After every task: `git status` should be clean and `cargo clippy --workspace -- -D warnings && cargo test --workspace` should pass before the commit. If clippy complains, fix inline.
- R package tests: `just rpkg-test` from repo root. If unavailable, `cd dvs-rpkg && R CMD INSTALL . && Rscript -e 'devtools::test()'`.
- Never skip hooks; never `--no-verify`.
- One commit per task unless a task explicitly says otherwise. Commit messages shown per task.

---

## File Structure

Files created:

- `dvs/src/batch.rs` — `BatchOutcome<S, E>` + `impl` methods. New module.
- `dvs-cli/src/output.rs` — new module holding the errors-first grouped output formatter (keeps `main.rs` lean). New module.

Files modified (core):

- `dvs/src/lib.rs` — re-export `BatchOutcome`, new `*Success`/`*Error` types, drop old `AddDetail`/`GetDetail`/`StatusDetail`/`AddResult`/`GetResult`/`FileStatus` re-exports.
- `dvs/src/files/add.rs` — add `AddSuccess`, `AddError`, port `add_files`.
- `dvs/src/files/get.rs` — add `GetSuccess`, `GetError`, port `get_files`.
- `dvs/src/files/status.rs` — add `StatusSuccess`, `StatusError`, port `get_status`.
- `dvs/src/globbing.rs` — `resolve_paths_for_add`/`_get` return `(Vec<PathBuf>, Vec<*Error>)`.

Files modified (CLI):

- `dvs-cli/src/main.rs` — consume new types, delegate human output to `output.rs`, set exit codes per spec.

Files modified (R bindings):

- `dvs-rpkg/src/rust/lib.rs` — `outcome_to_r` helper, `dvs_add_impl`/`dvs_get_impl`/`dvs_status_impl` return `List`.
- `dvs-rpkg/tests/testthat/test-add.R` — new.
- `dvs-rpkg/tests/testthat/test-get.R` — new.
- `dvs-rpkg/tests/testthat/test-status.R` — add batch-outcome assertions.

Files deleted (final cleanup task):

- Old types `AddResult`/`AddDetail`/`GetResult`/`GetDetail`/`FileStatus`/`StatusDetail` are removed from the `dvs` crate once all callers have migrated.

---

## Task 1: Introduce `BatchOutcome<S, E>` core type

**Files:**
- Create: `dvs/src/batch.rs`
- Modify: `dvs/src/lib.rs`

- [ ] **Step 1.1: Create the module with a failing test**

Create `dvs/src/batch.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Result of a batch operation that may partially succeed.
///
/// `S` is the success-row type, `E` the error-row type. Both are expected
/// to carry their own identifying field (e.g. `path`) in their payload; the
/// outcome itself is intentionally generic so it can be reused across commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutcome<S, E> {
    pub ok: Vec<S>,
    pub err: Vec<E>,
}

impl<S, E> BatchOutcome<S, E> {
    pub fn new() -> Self {
        Self {
            ok: Vec::new(),
            err: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.err.is_empty()
    }

    pub fn split(self) -> (Vec<S>, Vec<E>) {
        (self.ok, self.err)
    }
}

impl<S, E> Default for BatchOutcome<S, E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_clean_true_when_no_errors() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1, 2],
            err: vec![],
        };
        assert!(b.is_clean());
    }

    #[test]
    fn is_clean_false_when_any_error() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1],
            err: vec!["oops".into()],
        };
        assert!(!b.is_clean());
    }

    #[test]
    fn split_returns_both_vecs() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1, 2],
            err: vec!["e".into()],
        };
        let (ok, err) = b.split();
        assert_eq!(ok, vec![1, 2]);
        assert_eq!(err, vec!["e".to_string()]);
    }

    #[test]
    fn default_is_empty() {
        let b: BatchOutcome<u32, String> = BatchOutcome::default();
        assert!(b.ok.is_empty());
        assert!(b.err.is_empty());
        assert!(b.is_clean());
    }
}
```

- [ ] **Step 1.2: Wire the module into the crate**

Edit `dvs/src/lib.rs` — add at the top alongside the other `pub mod` declarations:

```rust
pub mod batch;
```

And in the `pub use` block, add:

```rust
pub use batch::BatchOutcome;
```

- [ ] **Step 1.3: Run the tests**

Run: `cargo test -p dvs batch::`
Expected: 4 tests pass.

- [ ] **Step 1.4: Commit**

```bash
git add dvs/src/batch.rs dvs/src/lib.rs
git commit -m "dvs: introduce BatchOutcome<S, E> core type

Generic partial-success/partial-failure result carrier.
Successor to the per-command Vec<*Result> shapes. No callers yet."
```

---

## Task 2: Add `AddSuccess` / `AddError` types (alongside existing)

Keep the old `AddResult` / `AddDetail` alive here — the port happens in Task 6. Defining the new types separately lets us write and commit their serde tests now.

**Files:**
- Modify: `dvs/src/files/add.rs`

- [ ] **Step 2.1: Append the new types to `add.rs`**

At the bottom of `dvs/src/files/add.rs` (above the `#[cfg(test)]` block), add:

```rust
/// Successful add of a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub hash: String,
    pub size: u64,
    pub stored_size: Option<u64>,
}

/// Structured failure for a single file in `add_files`.
/// Serializes with a `"kind"` tag so R / JSON consumers can group on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddError {
    /// Pre-flight: path does not exist on disk.
    NotFound       { path: PathBuf },
    /// Pre-flight: path escapes the repo root.
    OutsideProject { path: PathBuf },
    /// Pre-flight: path refers to a directory; `dvs` versions files only.
    IsDirectory    { path: PathBuf },
    /// Pre-flight: canonicalize / resolve failed.
    PathResolution { path: PathBuf, reason: String },
    /// Pre-flight: glob pattern could not be compiled or walked.
    GlobFailure    { pattern: String, reason: String },
    /// Runtime: hashing the local file failed.
    HashFailure    { path: PathBuf, reason: String },
    /// Runtime: writing the stored object / metadata failed.
    StorageWrite   { path: PathBuf, reason: String },
}

impl AddError {
    /// The path this error refers to, if any. `None` for `GlobFailure`.
    pub fn path(&self) -> Option<&Path> {
        match self {
            AddError::NotFound { path }
            | AddError::OutsideProject { path }
            | AddError::IsDirectory { path }
            | AddError::PathResolution { path, .. }
            | AddError::HashFailure { path, .. }
            | AddError::StorageWrite { path, .. } => Some(path.as_path()),
            AddError::GlobFailure { .. } => None,
        }
    }

    /// The serde `kind` string. Stable identifier, safe for scripting.
    pub fn kind(&self) -> &'static str {
        match self {
            AddError::NotFound { .. }       => "not_found",
            AddError::OutsideProject { .. } => "outside_project",
            AddError::IsDirectory { .. }    => "is_directory",
            AddError::PathResolution { .. } => "path_resolution",
            AddError::GlobFailure { .. }    => "glob_failure",
            AddError::HashFailure { .. }    => "hash_failure",
            AddError::StorageWrite { .. }   => "storage_write",
        }
    }
}
```

- [ ] **Step 2.2: Add serde tests for every `AddError` variant**

Inside the existing `#[cfg(test)] mod tests` block at the bottom of `add.rs`, add:

```rust
    #[test]
    fn add_error_serializes_with_snake_case_kind() {
        let cases: Vec<(AddError, &str)> = vec![
            (AddError::NotFound { path: "a.txt".into() }, "not_found"),
            (AddError::OutsideProject { path: "../x".into() }, "outside_project"),
            (AddError::IsDirectory { path: "data".into() }, "is_directory"),
            (AddError::PathResolution { path: "a".into(), reason: "x".into() }, "path_resolution"),
            (AddError::GlobFailure { pattern: "*.csv".into(), reason: "x".into() }, "glob_failure"),
            (AddError::HashFailure { path: "a".into(), reason: "x".into() }, "hash_failure"),
            (AddError::StorageWrite { path: "a".into(), reason: "x".into() }, "storage_write"),
        ];
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected_kind, "for variant {err:?}");
        }
    }

    #[test]
    fn add_error_path_accessor() {
        let e = AddError::NotFound { path: "a.txt".into() };
        assert_eq!(e.path().unwrap(), Path::new("a.txt"));

        let e = AddError::GlobFailure { pattern: "*".into(), reason: "x".into() };
        assert!(e.path().is_none());
    }

    #[test]
    fn add_success_roundtrips_json() {
        let s = AddSuccess {
            path: "a.txt".into(),
            outcome: Outcome::Copied,
            hash: "deadbeef".into(),
            size: 42,
            stored_size: Some(30),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AddSuccess = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hash, "deadbeef");
        assert_eq!(back.size, 42);
    }
```

- [ ] **Step 2.3: Run the tests**

Run: `cargo test -p dvs files::add::tests::add_error -- --nocapture && cargo test -p dvs files::add::tests::add_success`
Expected: 3 new tests pass; all existing `add_files_*` tests still pass.

- [ ] **Step 2.4: Export from crate root**

Edit `dvs/src/lib.rs`. Change:

```rust
pub use files::add::{AddDetail, AddResult, add_files};
```

to:

```rust
pub use files::add::{AddDetail, AddError, AddResult, AddSuccess, add_files};
```

(Keep the old re-exports alongside; they go away in Task 9.)

- [ ] **Step 2.5: Commit**

```bash
git add dvs/src/files/add.rs dvs/src/lib.rs
git commit -m "dvs: add structured AddSuccess and AddError types

Added alongside the existing AddResult/AddDetail; the port of
add_files() to BatchOutcome happens in a later commit."
```

---

## Task 3: Add `GetSuccess` / `GetError` types

**Files:**
- Modify: `dvs/src/files/get.rs`, `dvs/src/lib.rs`

- [ ] **Step 3.1: Append new types to `get.rs`**

In `dvs/src/files/get.rs` (above the `#[cfg(test)]` block):

```rust
/// Successful get of a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub size: u64,
}

/// Structured failure for a single file in `get_files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GetError {
    /// Pre-flight: user path does not resolve to any tracked file.
    NotFound       { path: PathBuf },
    /// Pre-flight: the file exists on disk but is not tracked by DVS.
    NotTracked     { path: PathBuf },
    /// Pre-flight: glob pattern could not be compiled.
    GlobFailure    { pattern: String, reason: String },
    /// Runtime: reading the `.dvs` metadata file failed.
    MetadataRead   { path: PathBuf, reason: String },
    /// Runtime: stored object is missing in the backend for the tracked hash.
    StorageMissing { path: PathBuf, hash: String },
    /// Runtime: backend retrieval failed.
    StorageRead    { path: PathBuf, reason: String },
    /// Runtime: retrieved object did not hash to the expected value.
    HashMismatch   { path: PathBuf, expected: String, got: String },
}

impl GetError {
    pub fn path(&self) -> Option<&Path> {
        match self {
            GetError::NotFound { path }
            | GetError::NotTracked { path }
            | GetError::MetadataRead { path, .. }
            | GetError::StorageMissing { path, .. }
            | GetError::StorageRead { path, .. }
            | GetError::HashMismatch { path, .. } => Some(path.as_path()),
            GetError::GlobFailure { .. } => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            GetError::NotFound { .. }       => "not_found",
            GetError::NotTracked { .. }     => "not_tracked",
            GetError::GlobFailure { .. }    => "glob_failure",
            GetError::MetadataRead { .. }   => "metadata_read",
            GetError::StorageMissing { .. } => "storage_missing",
            GetError::StorageRead { .. }    => "storage_read",
            GetError::HashMismatch { .. }   => "hash_mismatch",
        }
    }
}
```

- [ ] **Step 3.2: Serde tests**

Inside the existing `#[cfg(test)] mod tests` block in `get.rs`:

```rust
    #[test]
    fn get_error_serializes_with_snake_case_kind() {
        use super::{GetError};
        let cases: Vec<(GetError, &str)> = vec![
            (GetError::NotFound { path: "a".into() }, "not_found"),
            (GetError::NotTracked { path: "a".into() }, "not_tracked"),
            (GetError::GlobFailure { pattern: "*".into(), reason: "x".into() }, "glob_failure"),
            (GetError::MetadataRead { path: "a".into(), reason: "x".into() }, "metadata_read"),
            (GetError::StorageMissing { path: "a".into(), hash: "h".into() }, "storage_missing"),
            (GetError::StorageRead { path: "a".into(), reason: "x".into() }, "storage_read"),
            (GetError::HashMismatch { path: "a".into(), expected: "x".into(), got: "y".into() }, "hash_mismatch"),
        ];
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected_kind);
        }
    }

    #[test]
    fn get_success_roundtrips_json() {
        use super::GetSuccess;
        let s = GetSuccess { path: "a".into(), outcome: Outcome::Copied, size: 10 };
        let back: GetSuccess = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.size, 10);
    }
```

- [ ] **Step 3.3: Run**

Run: `cargo test -p dvs files::get::tests::get_error && cargo test -p dvs files::get::tests::get_success`
Expected: both new tests pass.

- [ ] **Step 3.4: Export**

Edit `dvs/src/lib.rs`. Change:

```rust
pub use files::get::{GetDetail, GetResult, get_files};
```

to:

```rust
pub use files::get::{GetDetail, GetError, GetResult, GetSuccess, get_files};
```

- [ ] **Step 3.5: Commit**

```bash
git add dvs/src/files/get.rs dvs/src/lib.rs
git commit -m "dvs: add structured GetSuccess and GetError types"
```

---

## Task 4: Add `StatusSuccess` / `StatusError` types

**Files:**
- Modify: `dvs/src/files/status.rs`, `dvs/src/lib.rs`

- [ ] **Step 4.1: Append new types to `status.rs`**

In `dvs/src/files/status.rs`, above the `#[cfg(test)]` block:

```rust
/// Successful status row for a single tracked file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusSuccess {
    pub path: PathBuf,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

/// Structured failure for a single file in `get_status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StatusError {
    /// Internal: could not determine the path relative to the `.dvs` folder.
    RelativePath { metadata_path: PathBuf, reason: String },
    /// Reading the `.dvs` metadata file failed.
    MetadataRead { path: PathBuf, reason: String },
    /// Hashing the on-disk file to compare against metadata failed.
    HashFailure  { path: PathBuf, reason: String },
}

impl StatusError {
    pub fn kind(&self) -> &'static str {
        match self {
            StatusError::RelativePath { .. } => "relative_path",
            StatusError::MetadataRead { .. } => "metadata_read",
            StatusError::HashFailure { .. }  => "hash_failure",
        }
    }
}
```

- [ ] **Step 4.2: Serde tests**

Inside the existing `#[cfg(test)] mod tests` block in `status.rs`:

```rust
    #[test]
    fn status_error_serializes_with_snake_case_kind() {
        use super::StatusError;
        let cases: Vec<(StatusError, &str)> = vec![
            (StatusError::RelativePath { metadata_path: "a".into(), reason: "x".into() }, "relative_path"),
            (StatusError::MetadataRead { path: "a".into(), reason: "x".into() }, "metadata_read"),
            (StatusError::HashFailure { path: "a".into(), reason: "x".into() }, "hash_failure"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.kind(), expected);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected);
        }
    }
```

- [ ] **Step 4.3: Run**

Run: `cargo test -p dvs files::status::tests::status_error`
Expected: passes.

- [ ] **Step 4.4: Export**

Edit `dvs/src/lib.rs`. Change:

```rust
pub use files::status::{FileStatus, StatusDetail, StatusFilter, get_status};
```

to:

```rust
pub use files::status::{
    FileStatus, StatusDetail, StatusError, StatusFilter, StatusSuccess, get_status,
};
```

- [ ] **Step 4.5: Commit**

```bash
git add dvs/src/files/status.rs dvs/src/lib.rs
git commit -m "dvs: add structured StatusSuccess and StatusError types"
```

---

## Task 5: Refactor glob resolution to emit structured errors

Changes `resolve_paths_for_add` and `resolve_paths_for_get` from `Result<HashSet<PathBuf>>` (fail-fast) to `(Vec<PathBuf>, Vec<AddError>)` / `(Vec<PathBuf>, Vec<GetError>)`. Callers get resolvable inputs *and* a list of failures in one pass.

**Files:**
- Modify: `dvs/src/globbing.rs`

- [ ] **Step 5.1: Rewrite `resolve_paths_for_add`**

Replace the function in `dvs/src/globbing.rs` with:

```rust
pub fn resolve_paths_for_add(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> (Vec<PathBuf>, Vec<crate::AddError>) {
    use crate::AddError;

    let mut out: HashSet<PathBuf> = HashSet::new();
    let mut errs: Vec<AddError> = Vec::new();

    let glob_matcher = match build_glob_matcher(glob_pattern) {
        Ok(m) => m,
        Err(e) => {
            errs.push(AddError::GlobFailure {
                pattern: glob_pattern.unwrap_or_default().to_string(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };

    let repo_root = match dvs_paths.repo_root().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(AddError::PathResolution {
                path: dvs_paths.repo_root().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let metadata_root = match dvs_paths.metadata_folder().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(AddError::PathResolution {
                path: dvs_paths.metadata_folder().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };

    let paths = if paths.is_empty() { vec![PathBuf::from(".")] } else { paths };

    for path in paths {
        let full_path = match dvs_paths.cwd().join(&path).canonicalize() {
            Ok(p) => p,
            Err(_) => {
                errs.push(AddError::NotFound { path: path.clone() });
                continue;
            }
        };

        if full_path.is_file() {
            let relative_to_root = match full_path.strip_prefix(&repo_root) {
                Ok(p) => p.to_path_buf(),
                Err(_) => path.clone(), // validate_for_add will flag OutsideProject later
            };
            out.insert(relative_to_root);
        } else if full_path.is_dir() {
            if let Some(matcher) = &glob_matcher {
                for entry in WalkDir::new(&full_path).into_iter().filter_map(|e| e.ok()) {
                    let entry_path = match entry.path().canonicalize() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if !entry_path.is_file() || entry_path.starts_with(&metadata_root) {
                        continue;
                    }
                    let relative_to_dir = match entry_path.strip_prefix(&full_path) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if matcher.is_match(relative_to_dir) {
                        if let Ok(rel) = entry_path.strip_prefix(&repo_root) {
                            out.insert(rel.to_path_buf());
                        }
                    }
                }
            }
            // If no glob, walking a directory contributes nothing; this is
            // not an error — the user just gets an empty result for that dir.
        } else {
            // Special file (socket, device). Classify as PathResolution.
            errs.push(AddError::PathResolution {
                path: path.clone(),
                reason: "not a regular file or directory".to_string(),
            });
        }
    }

    let mut ok: Vec<PathBuf> = out.into_iter().collect();
    ok.sort();
    (ok, errs)
}
```

- [ ] **Step 5.2: Rewrite `resolve_paths_for_get` analogously**

Replace `resolve_paths_for_get` in the same file:

```rust
pub fn resolve_paths_for_get(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> (Vec<PathBuf>, Vec<crate::GetError>) {
    use crate::GetError;

    let mut out: HashSet<PathBuf> = HashSet::new();
    let mut errs: Vec<GetError> = Vec::new();

    let glob_matcher = match build_glob_matcher(glob_pattern) {
        Ok(m) => m,
        Err(e) => {
            errs.push(GetError::GlobFailure {
                pattern: glob_pattern.unwrap_or_default().to_string(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let metadata_root = match dvs_paths.metadata_folder().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(GetError::MetadataRead {
                path: dvs_paths.metadata_folder().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let cwd_prefix = dvs_paths.cwd_relative_to_root();

    // Convert user paths to repo-relative directory filters.
    // If no paths given, default to cwd (or repo root if at root).
    let dir_filters: Vec<PathBuf> = if paths.is_empty() {
        vec![cwd_prefix.map(|p| p.to_path_buf()).unwrap_or_default()]
    } else {
        paths
            .into_iter()
            .map(|p| {
                if p.is_absolute() {
                    match p.strip_prefix(dvs_paths.repo_root()) {
                        Ok(r) => r.to_path_buf(),
                        Err(_) => p,
                    }
                } else if let Some(prefix) = cwd_prefix {
                    prefix.join(&p)
                } else {
                    p
                }
            })
            .collect()
    };

    for entry in WalkDir::new(&metadata_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() { continue; }
        let entry_path = entry.path();
        if entry_path.extension() != Some(OsStr::new("dvs")) { continue; }

        let repo_relative = match entry_path.strip_prefix(&metadata_root) {
            Ok(p) => p.with_extension(""),
            Err(_) => continue,
        };

        // Directory filter: path must match at least one filter or be under it.
        let in_scope = dir_filters.iter().any(|f| {
            f.as_os_str().is_empty()
                || repo_relative == *f
                || repo_relative.starts_with(f)
        });
        if !in_scope { continue; }

        if let Some(matcher) = &glob_matcher {
            if !matcher.is_match(&repo_relative) { continue; }
        }
        out.insert(repo_relative);
    }

    let mut ok: Vec<PathBuf> = out.into_iter().collect();
    ok.sort();
    (ok, errs)
}
```

> **Implementer:** the exact directory-filtering logic above is a best-effort reconstruction of the current `resolve_paths_for_get` semantics — compare against the existing function body *before* replacing, and if there are behavioral details (e.g. non-absolute-path handling edge cases) not captured here, preserve them. The mandatory changes are only: (a) return `(Vec<PathBuf>, Vec<GetError>)` instead of `Result<HashSet>`; (b) every `bail!` or `?` on a recoverable per-path/per-pattern failure becomes `errs.push(GetError::…)` and `continue`.

- [ ] **Step 5.3: Update unit tests in `globbing.rs` (if any) and add new ones**

At the bottom of `dvs/src/globbing.rs`, ensure there is a `#[cfg(test)] mod tests` block with at least:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};

    fn make_paths(root: &std::path::Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(root.to_path_buf(), root.to_path_buf(), config.metadata_folder_name()).unwrap()
    }

    #[test]
    fn resolve_add_missing_path_becomes_error_row() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        let (ok, errs) = resolve_paths_for_add(
            vec!["does-not-exist.csv".into()],
            None,
            &paths,
        );
        assert!(ok.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], crate::AddError::NotFound { .. }));
    }

    #[test]
    fn resolve_add_mixed_inputs_collect_all_errors() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        create_file(&root, "ok.csv", b"a");

        let (ok, errs) = resolve_paths_for_add(
            vec!["ok.csv".into(), "missing1.csv".into(), "missing2.csv".into()],
            None,
            &paths,
        );
        assert_eq!(ok.len(), 1);
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().all(|e| matches!(e, crate::AddError::NotFound { .. })));
    }

    #[test]
    fn resolve_add_malformed_glob_becomes_error() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);

        let (ok, errs) = resolve_paths_for_add(
            vec![],
            Some("[invalid"),
            &paths,
        );
        assert!(ok.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], crate::AddError::GlobFailure { .. }));
    }
}
```

- [ ] **Step 5.4: Fix all downstream callers that depended on the old `Result<HashSet>` return**

Callers to patch (they currently use `?` on the function):

- `dvs-cli/src/main.rs` lines 240, 407 — use of `resolve_paths_for_add`/`_get`.
- `dvs-rpkg/src/rust/lib.rs` lines ~221, ~445.

**In all four spots**, replace:

```rust
let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
    .into_iter()
    .collect();
if all_paths.is_empty() {
    return Err(anyhow!("No files to add"));
}
```

with a **temporary** adapter that preserves old behavior until the caller is ported in later tasks:

```rust
let (all_paths, resolve_errs) = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths);
if !resolve_errs.is_empty() {
    // Temporary: caller not yet migrated to BatchOutcome. Collapse to anyhow.
    return Err(anyhow!(
        "path resolution failed for {} input(s): {}",
        resolve_errs.len(),
        resolve_errs.iter()
            .map(|e| e.kind())
            .collect::<Vec<_>>()
            .join(", ")
    ));
}
if all_paths.is_empty() {
    return Err(anyhow!("No files to add"));
}
```

Apply the same adapter (with `GetError`) at the two `resolve_paths_for_get` call sites.

- [ ] **Step 5.5: Verify the workspace still builds and tests pass**

Run: `cargo build --workspace && cargo test --workspace`
Expected: all green. Pre-existing tests that asserted `resolve_paths_for_*` returned `Err` for invalid inputs will need updating to read the new shape — touch them here.

- [ ] **Step 5.6: Commit**

```bash
git add dvs/src/globbing.rs dvs-cli/src/main.rs dvs-rpkg/src/rust/lib.rs
git commit -m "dvs: glob resolution returns (Vec<PathBuf>, Vec<*Error>)

Pre-flight path and pattern failures become structured error rows
instead of fail-fast bail!. Callers temporarily collapse the err vec
back into anyhow::Err; those adapters are removed in later commits
when each caller is migrated to BatchOutcome."
```

---

## Task 6: Port `add_files` to return `BatchOutcome<AddSuccess, AddError>`, update CLI and R consumers

Single commit, atomic port. After this task the `add` path is fully on the new types; `dvs-cli add` emits errors-first grouped output; R `dvs_add` returns `list(ok, err)`.

**Files:**
- Modify: `dvs/src/files/add.rs`, `dvs/src/lib.rs`
- Create: `dvs-cli/src/output.rs`
- Modify: `dvs-cli/src/main.rs`
- Modify: `dvs-rpkg/src/rust/lib.rs`
- Modify: `dvs-rpkg/tests/testthat/test-add.R` (new file)

- [ ] **Step 6.1: Port `add_files` signature and body**

Replace the `add_files` function in `dvs/src/files/add.rs`. First, define a module-private `Row` enum at file scope (just below the existing `AddError` impl), and the classifier:

```rust
#[derive(Debug)]
enum Row { Ok(AddSuccess), Err(AddError) }

/// Bucket a runtime `anyhow::Error` from `add_file()` into the closest-fitting
/// `AddError` variant. Default bucket is `StorageWrite` — the write path is
/// the most common runtime failure source.
fn classify_add_runtime_err(path: PathBuf, err: anyhow::Error) -> Row {
    let msg = err.to_string();
    if msg.contains("hash") || msg.contains("blake3") {
        Row::Err(AddError::HashFailure { path, reason: msg })
    } else {
        Row::Err(AddError::StorageWrite { path, reason: msg })
    }
}
```

Then replace the function body:

```rust
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<crate::BatchOutcome<AddSuccess, AddError>> {
    let matched_paths = paths.validate_for_add(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    let mut rows: Vec<Row> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, status)| match status {
                AddPathStatus::NotFound =>
                    Row::Err(AddError::NotFound { path: relative_path }),
                AddPathStatus::OutsideProject =>
                    Row::Err(AddError::OutsideProject { path: relative_path }),
                AddPathStatus::IsDirectory =>
                    Row::Err(AddError::IsDirectory { path: relative_path }),
                AddPathStatus::Valid => {
                    let full_path = paths.file_path(&relative_path);
                    match full_path.canonicalize() {
                        Ok(canonical) if !canonical.starts_with(paths.repo_root()) =>
                            return Row::Err(AddError::OutsideProject { path: relative_path }),
                        Err(e) =>
                            return Row::Err(AddError::PathResolution {
                                path: relative_path,
                                reason: e.to_string(),
                            }),
                        _ => {}
                    }
                    let file_size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                    let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                    let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
                    match add_file(
                        &relative_path,
                        paths,
                        backend,
                        cache.as_ref(),
                        operation_id,
                        message.clone(),
                        compression,
                        dry_run,
                        on_bytes,
                    ) {
                        Ok((outcome, metadata, stored_size)) => Row::Ok(AddSuccess {
                            path: relative_path,
                            outcome,
                            hash: metadata.hashes.blake3,
                            size: metadata.size,
                            stored_size,
                        }),
                        Err(e) => classify_add_runtime_err(relative_path, e),
                    }
                }
            })
            .map(Some)
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect()
    });

    rows.sort_by(|a, b| {
        let ak = match a { Row::Ok(s) => s.path.as_path(), Row::Err(e) => e.path().unwrap_or(std::path::Path::new("")) };
        let bk = match b { Row::Ok(s) => s.path.as_path(), Row::Err(e) => e.path().unwrap_or(std::path::Path::new("")) };
        ak.cmp(bk)
    });

    let mut outcome = crate::BatchOutcome::<AddSuccess, AddError>::new();
    for row in rows {
        match row {
            Row::Ok(s)  => outcome.ok.push(s),
            Row::Err(e) => outcome.err.push(e),
        }
    }

    let successful_paths: Vec<_> = outcome.ok.iter().map(|s| s.path.clone()).collect();
    if !dry_run && !successful_paths.is_empty() {
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(outcome)
}
```

The `Row` enum and `classify_add_runtime_err` from the top of this step are referenced from inside the closure; because `Row` is declared at module scope, both the closure and the classifier see the same type.

- [ ] **Step 6.2: Update the existing unit tests in `add.rs`**

Change the existing `add_files_reports_not_found_per_file` and `add_files_mixed_statuses` tests so they:

1. Call `.unwrap()` on `Result<BatchOutcome<_,_>>`.
2. Assert on `outcome.ok` and `outcome.err` independently.
3. Use `matches!(err, AddError::NotFound { .. })` style, no string matching.

Replace them in full (block-replace both functions):

```rust
    #[test]
    fn add_files_reports_not_found_per_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let outcome = add_files(
            vec!["nonexistent.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();

        assert!(outcome.ok.is_empty());
        assert_eq!(outcome.err.len(), 1);
        assert!(matches!(outcome.err[0], AddError::NotFound { .. }));
    }

    #[test]
    fn add_files_mixed_statuses() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let outside_tmp = tempfile::tempdir().unwrap();
        let outside_file = fs::canonicalize(outside_tmp.path())
            .unwrap()
            .join("outside.txt");
        fs::write(&outside_file, b"outside").unwrap();
        let outside_relative =
            PathBuf::from("..").join(outside_file.strip_prefix(root.parent().unwrap()).unwrap());

        fs::create_dir(root.join("subdir")).unwrap();

        let outcome = add_files(
            vec![
                "a.txt".into(),
                "missing.csv".into(),
                outside_relative,
                "subdir".into(),
            ],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();

        assert_eq!(outcome.ok.len(), 1);
        assert_eq!(outcome.ok[0].path, PathBuf::from("a.txt"));
        assert_eq!(outcome.ok[0].outcome, Outcome::Copied);

        assert_eq!(outcome.err.len(), 3);
        assert!(outcome.err.iter().any(|e| matches!(e, AddError::NotFound { .. })));
        assert!(outcome.err.iter().any(|e| matches!(e, AddError::OutsideProject { .. })));
        assert!(outcome.err.iter().any(|e| matches!(e, AddError::IsDirectory { .. })));
    }
```

- [ ] **Step 6.3: Run core tests**

Run: `cargo test -p dvs files::add`
Expected: all pass, including the two updated ones.

- [ ] **Step 6.4: Create the CLI output module**

Create `dvs-cli/src/output.rs`:

```rust
use std::collections::BTreeMap;
use std::path::Path;

use dvs::{AddError, AddSuccess, GetError, GetSuccess, StatusError, StatusSuccess};

/// Print `add` results to stdout/stderr in the spec's errors-first layout.
/// Returns true if any error rows were present.
pub fn print_add(
    ok: &[AddSuccess],
    err: &[AddError],
    dry_run: bool,
    format_size: impl Fn(u64) -> String,
) -> bool {
    if !err.is_empty() {
        print_add_failures(err);
        eprintln!();
    }
    for s in ok {
        let stored_info = match s.stored_size {
            Some(ss) => format!(" --> saved [{}]", format_size(ss)),
            None => String::new(),
        };
        match s.outcome {
            dvs::Outcome::Copied => {
                let verb = if dry_run { "To add" } else { "Added" };
                println!(
                    "{verb}: {} [{}]{stored_info} as {}",
                    s.path.display(),
                    format_size(s.size),
                    s.hash,
                );
            }
            dvs::Outcome::Present => {}
        }
    }
    !err.is_empty()
}

fn print_add_failures(err: &[AddError]) {
    let mut groups: BTreeMap<&'static str, Vec<&AddError>> = BTreeMap::new();
    for e in err {
        groups.entry(e.kind()).or_default().push(e);
    }
    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        let mut paths: Vec<String> = items.iter().filter_map(|e| {
            match e {
                AddError::GlobFailure { pattern, reason } =>
                    Some(format!("glob {pattern:?}: {reason}")),
                _ => e.path().map(|p| p.display().to_string()),
            }
        }).collect();
        paths.sort();
        for p in paths { eprintln!("    {p}"); }
    }
}

pub fn print_get(
    ok: &[GetSuccess],
    err: &[GetError],
    format_size: impl Fn(u64) -> String,
) -> bool {
    if !err.is_empty() {
        print_get_failures(err);
        eprintln!();
    }
    let mut total_files = 0u64;
    let mut total_bytes = 0u64;
    for s in ok {
        if s.outcome == dvs::Outcome::Copied {
            println!("{} [{}]", s.path.display(), format_size(s.size));
            total_files += 1;
            total_bytes += s.size;
        }
    }
    if total_files > 0 {
        println!("Total: {} files, {}", total_files, format_size(total_bytes));
    }
    !err.is_empty()
}

fn print_get_failures(err: &[GetError]) {
    let mut groups: BTreeMap<&'static str, Vec<&GetError>> = BTreeMap::new();
    for e in err { groups.entry(e.kind()).or_default().push(e); }
    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        let mut paths: Vec<String> = items.iter().filter_map(|e| match e {
            GetError::GlobFailure { pattern, reason } =>
                Some(format!("glob {pattern:?}: {reason}")),
            GetError::HashMismatch { path, expected, got } =>
                Some(format!("{} (expected {}, got {})", path.display(), expected, got)),
            _ => e.path().map(|p| p.display().to_string()),
        }).collect();
        paths.sort();
        for p in paths { eprintln!("    {p}"); }
    }
}

pub fn print_status_failures(err: &[StatusError]) {
    if err.is_empty() { return; }
    let mut groups: BTreeMap<&'static str, Vec<&StatusError>> = BTreeMap::new();
    for e in err { groups.entry(e.kind()).or_default().push(e); }
    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        for e in items {
            let line = match e {
                StatusError::RelativePath { metadata_path, reason } =>
                    format!("{}: {reason}", metadata_path.display()),
                StatusError::MetadataRead { path, reason } =>
                    format!("{}: {reason}", path.display()),
                StatusError::HashFailure { path, reason } =>
                    format!("{}: {reason}", path.display()),
            };
            eprintln!("    {line}");
        }
    }
    eprintln!();
}

// Silence unused-import until Task 8.
#[allow(dead_code)]
fn _force_use_status_success(_: &StatusSuccess) -> &Path { Path::new("") }
```

And declare the module in `dvs-cli/src/main.rs` at the top:

```rust
mod output;
```

- [ ] **Step 6.5: Wire the Add branch of `dvs-cli` to the new flow**

In `dvs-cli/src/main.rs`, replace the entire `Command::Add { .. }` match arm body (lines ~231–302) with:

```rust
        Command::Add { paths, glob, message, dry_run } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;

            let (all_paths, mut resolve_errs) =
                resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths);
            if all_paths.is_empty() && resolve_errs.is_empty() {
                return Err(anyhow!("No files to add"));
            }

            let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();
            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
            let mut outcome = add_files(
                all_paths,
                &dvs_paths,
                config.backend(),
                message,
                config.compression(),
                dry_run,
                if show_progress { Some(&on_file_start) } else { None },
            )?;
            outcome.err.append(&mut resolve_errs);

            if cli.json {
                println!("{}", serde_json::to_string(&outcome)?);
            } else {
                output::print_add(&outcome.ok, &outcome.err, dry_run, format_size);
            }

            if !outcome.err.is_empty() {
                std::process::exit(1);
            }
        }
```

Also update the imports at the top of `main.rs`:

```rust
use dvs::{
    AddError, AddSuccess, Compression, FileMetadata, FileProgress, GetError, GetSuccess,
    Outcome, Status, StatusError, StatusSuccess, StatusFilter,
    add_files, format_size, get_files, get_status, set_num_threads,
};
```

(Keep the old `AddDetail`/`GetDetail`/`StatusDetail` imports for now; Tasks 7 and 8 will drop them.)

- [ ] **Step 6.6: R bindings — `outcome_to_r` helper and `dvs_add_impl`**

In `dvs-rpkg/src/rust/lib.rs`, add near the top (after existing imports):

```rust
use dvs::BatchOutcome;
```

Add this helper function above the `// region: DVS operations` marker. Returns `anyhow::Result<List>` so callers can use `?` alongside other anyhow-returning code:

```rust
/// Convert a `BatchOutcome` to an R list.
///
/// `$ok` is always present (zero-row data frame if no successes) so that
/// callers can index `$ok` unconditionally. `$err` is present only when
/// there are failures — per the spec, only the populated side is returned
/// when one is empty.
fn outcome_to_r<S, E>(outcome: BatchOutcome<S, E>) -> Result<List>
where
    S: Serialize,
    E: Serialize,
{
    let ok_df = miniextendr_api::serde::vec_to_dataframe(&outcome.ok)
        .map_err(|e| anyhow!("serializing ok rows: {e}"))?;
    let mut l = list!("ok" = ok_df);
    if !outcome.err.is_empty() {
        let err_df = miniextendr_api::serde::vec_to_dataframe(&outcome.err)
            .map_err(|e| anyhow!("serializing err rows: {e}"))?;
        l.set("err", err_df)
            .map_err(|e| anyhow!("setting err on list: {e}"))?;
    }
    Ok(l)
}
```

> **Implementer — the `List` method for "add a named element":** `miniextendr_api::List` exposes either `set(name, value)` or `set_named(name, value)` depending on the vendored version. Confirm by `rg 'impl .* List' dvs-rpkg/vendor/miniextendr-api`. If the method is spelled differently, substitute. The return shape (anyhow::Result<List> containing `ok` + optional `err`) is the invariant; the exact binding method is a syntactic detail.

Replace `dvs_add` in `dvs-rpkg/src/rust/lib.rs` (the function starting at ~line 210) with:

```rust
#[miniextendr(r_name = "dvs_add_impl")]
pub(crate) fn dvs_add(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] message: Option<String>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
) -> Result<List> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let (all_paths, mut resolve_errs) =
        resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths);
    if all_paths.is_empty() && resolve_errs.is_empty() {
        return Err(anyhow!("No files to add"));
    }

    let dry_run = dry_run.unwrap_or(false);
    let mut outcome = if progress_callback.is_some() {
        run_with_progress(|tx| {
            let on_file_start = progress_on_file_start(tx);
            add_files(
                all_paths.clone(),
                &dvs_paths,
                config.backend(),
                message.clone(),
                config.compression(),
                dry_run,
                Some(&on_file_start),
            )
        })?
    } else {
        add_files(
            all_paths,
            &dvs_paths,
            config.backend(),
            message,
            config.compression(),
            dry_run,
            None,
        )?
    };
    outcome.err.append(&mut resolve_errs);
    outcome_to_r(outcome)
}
```

- [ ] **Step 6.7: R test — `test-add.R`**

Create `dvs-rpkg/tests/testthat/test-add.R`:

```r
test_that("dvs_add returns only $ok when all paths succeed", {
  skip_if_not(nzchar(Sys.which("Rscript")))
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("hello", "a.csv")
  res <- dvs::dvs_add("a.csv")

  expect_true(!is.null(res$ok))
  expect_null(res$err)
  expect_equal(nrow(res$ok), 1L)
  expect_equal(res$ok$path, "a.csv")
})

test_that("dvs_add returns $ok and $err with kind column on mixed inputs", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "ok.csv")
  res <- dvs::dvs_add(c("ok.csv", "missing.csv"))

  expect_equal(nrow(res$ok), 1L)
  expect_equal(nrow(res$err), 1L)
  expect_true("kind" %in% names(res$err))
  expect_equal(res$err$kind, "not_found")
})

test_that("dvs_add returns zero-row $ok and populated $err when all fail", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  res <- dvs::dvs_add(c("nope1.csv", "nope2.csv"))

  expect_equal(nrow(res$ok), 0L)
  expect_equal(nrow(res$err), 2L)
  expect_true(all(res$err$kind == "not_found"))
})
```

- [ ] **Step 6.8: Run all tests**

```bash
cargo test --workspace
just rpkg-configure && just rpkg-install && just rpkg-test 2>&1 > /tmp/rpkg.log
```

Expected: all Rust + all R tests pass. If `dvs-commands.R` wraps `dvs_add_impl` and the old wrapper asserts on a data frame return, also update that file so the wrapper returns the list directly.

- [ ] **Step 6.9: Commit**

```bash
git add dvs/src/files/add.rs dvs-cli/src/main.rs dvs-cli/src/output.rs dvs-rpkg/src/rust/lib.rs dvs-rpkg/tests/testthat/test-add.R dvs-rpkg/R/dvs-commands.R
git commit -m "dvs,dvs-cli,dvs-rpkg: port 'add' to BatchOutcome

add_files now returns BatchOutcome<AddSuccess, AddError>. CLI prints
errors-first grouped output and exits 1 on any error row. R binding
returns list(ok, err) via outcome_to_r helper, \$err only when present."
```

---

## Task 7: Port `get_files` (same pattern as Task 6)

**Files:** `dvs/src/files/get.rs`, `dvs/src/lib.rs`, `dvs-cli/src/main.rs`, `dvs-rpkg/src/rust/lib.rs`, `dvs-rpkg/tests/testthat/test-get.R`

- [ ] **Step 7.1: Port `get_files` signature**

Replace `get_files` in `dvs/src/files/get.rs`. First, at module scope (same pattern as Task 6.1), add:

```rust
enum Row { Ok(GetSuccess), Err(GetError) }

fn classify_get_runtime_err(path: PathBuf, err: anyhow::Error) -> Row {
    let msg = err.to_string();
    if msg.contains("is not tracked") {
        Row::Err(GetError::NotTracked { path })
    } else if msg.starts_with("Storage file missing") {
        let hash = msg.rsplit(':').next().unwrap_or("").trim().to_string();
        Row::Err(GetError::StorageMissing { path, hash })
    } else if msg.contains("does not match expected hash") {
        Row::Err(GetError::HashMismatch { path, expected: String::new(), got: String::new() })
    } else if msg.contains("metadata") || msg.contains("serde_json") {
        Row::Err(GetError::MetadataRead { path, reason: msg })
    } else {
        Row::Err(GetError::StorageRead { path, reason: msg })
    }
}
```

Then replace the function:

```rust
pub fn get_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<crate::BatchOutcome<GetSuccess, GetError>> {
    let matched_paths = paths.validate_for_get(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);

    enum Row { Ok(GetSuccess), Err(GetError) }

    let mut rows: Vec<Row> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, validation)| match validation {
                GetPathStatus::NotFound   => Row::Err(GetError::NotFound   { path: relative_path }),
                GetPathStatus::NotTracked => Row::Err(GetError::NotTracked { path: relative_path }),
                GetPathStatus::Tracked => {
                    let file_size = {
                        let meta_path = paths.metadata_path(&relative_path);
                        std::fs::File::open(&meta_path)
                            .ok()
                            .and_then(|f| serde_json::from_reader::<_, FileMetadata>(f).ok())
                            .map(|m| m.size)
                            .unwrap_or(0)
                    };
                    let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                    let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);

                    match get_file(backend, paths, &relative_path, cache.as_ref(), dry_run, on_bytes) {
                        Ok((outcome, size)) => Row::Ok(GetSuccess {
                            path: relative_path,
                            outcome,
                            size,
                        }),
                        Err(e) => classify_get_runtime_err(relative_path, e),
                    }
                }
            })
            .collect()
    });

    rows.sort_by(|a, b| {
        let ak = match a { Row::Ok(s) => s.path.as_path(), Row::Err(e) => e.path().unwrap_or(std::path::Path::new("")) };
        let bk = match b { Row::Ok(s) => s.path.as_path(), Row::Err(e) => e.path().unwrap_or(std::path::Path::new("")) };
        ak.cmp(bk)
    });

    let mut outcome = crate::BatchOutcome::<GetSuccess, GetError>::new();
    for row in rows {
        match row {
            Row::Ok(s)  => outcome.ok.push(s),
            Row::Err(e) => outcome.err.push(e),
        }
    }
    Ok(outcome)
}
```

(The `Row` enum and `classify_get_runtime_err` from the top of this step are at module scope; the closure inside `par_iter` and the classifier both reference the same type.)

- [ ] **Step 7.2: Update existing `get.rs` tests**

Replace `get_files_reports_not_found_per_file` and `get_files_reports_not_tracked_for_untracked_file` and the `run_add_get_roundtrip` helper's assertions to use `outcome.ok` / `outcome.err` with typed-variant `matches!`. Mechanically:

- Where the old code called `results.iter().find(|r| ...)` and asserted on `r.detail`, now find inside `outcome.err` and `matches!(e, GetError::NotFound { .. })` etc.
- Where the old code asserted `GetDetail::Success { outcome: Outcome::Copied, .. }`, now assert `outcome.ok[i].outcome == Outcome::Copied`.
- In `add_get_roundtrip_with_explicit_paths`, also update the add-side call to use `outcome.ok.len()` instead of `results.len()`.

- [ ] **Step 7.3: Update CLI Get branch**

In `dvs-cli/src/main.rs`, replace the `Command::Get { .. }` match arm (lines ~399–457) with:

```rust
        Command::Get { paths, glob, dry_run } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;

            let (all_paths, mut resolve_errs) =
                resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths);
            if all_paths.is_empty() && resolve_errs.is_empty() {
                return Err(anyhow!("No files to get"));
            }

            let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();
            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
            let mut outcome = get_files(
                all_paths,
                &dvs_paths,
                config.backend(),
                dry_run,
                if show_progress { Some(&on_file_start) } else { None },
            )?;
            outcome.err.append(&mut resolve_errs);

            if cli.json {
                println!("{}", serde_json::to_string(&outcome)?);
            } else {
                output::print_get(&outcome.ok, &outcome.err, format_size);
            }

            if !outcome.err.is_empty() {
                std::process::exit(1);
            }
        }
```

Drop the `GetDetail` import.

- [ ] **Step 7.4: R binding for `dvs_get_impl`**

Replace the body of `dvs_get` in `dvs-rpkg/src/rust/lib.rs` with the BatchOutcome version (analogous to Task 6.6):

```rust
#[miniextendr(r_name = "dvs_get_impl")]
pub(crate) fn dvs_get(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] dry_run: Option<bool>,
    #[miniextendr(default = "NULL")] progress_callback: Option<ExternalPtr<ProgressBarCallback>>,
) -> Result<List> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let (all_paths, mut resolve_errs) =
        resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths);
    if all_paths.is_empty() && resolve_errs.is_empty() {
        return Err(anyhow!("No files to get"));
    }

    let dry_run = dry_run.unwrap_or(false);
    let mut outcome = if progress_callback.is_some() {
        run_with_progress(|tx| {
            let on_file_start = progress_on_file_start(tx);
            get_files(all_paths.clone(), &dvs_paths, config.backend(), dry_run, Some(&on_file_start))
        })?
    } else {
        get_files(all_paths, &dvs_paths, config.backend(), dry_run, None)?
    };
    outcome.err.append(&mut resolve_errs);
    outcome_to_r(outcome)
}
```

- [ ] **Step 7.5: Create `test-get.R`**

Create `dvs-rpkg/tests/testthat/test-get.R`:

```r
test_that("dvs_get returns only $ok after successful add + local delete", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("data", "a.csv")
  dvs::dvs_add("a.csv")
  file.remove("a.csv")

  res <- dvs::dvs_get("a.csv")
  expect_equal(nrow(res$ok), 1L)
  expect_null(res$err)
})

test_that("dvs_get returns $err with kind=not_tracked for untracked path", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "untracked.csv")
  res <- dvs::dvs_get("untracked.csv")

  expect_equal(nrow(res$ok), 0L)
  expect_equal(nrow(res$err), 1L)
  expect_equal(res$err$kind, "not_tracked")
})
```

- [ ] **Step 7.6: Run all tests**

```bash
cargo test --workspace
just rpkg-test 2>&1 > /tmp/rpkg.log
```

All green.

- [ ] **Step 7.7: Commit**

```bash
git add dvs/src/files/get.rs dvs-cli/src/main.rs dvs-rpkg/src/rust/lib.rs dvs-rpkg/tests/testthat/test-get.R dvs-rpkg/R/dvs-commands.R
git commit -m "dvs,dvs-cli,dvs-rpkg: port 'get' to BatchOutcome"
```

---

## Task 8: Port `get_status` (same pattern)

**Files:** `dvs/src/files/status.rs`, `dvs/src/lib.rs`, `dvs-cli/src/main.rs`, `dvs-rpkg/src/rust/lib.rs`, `dvs-rpkg/tests/testthat/test-status.R`

- [ ] **Step 8.1: Port `get_status`**

Replace `get_status` in `dvs/src/files/status.rs` with:

```rust
pub fn get_status(
    paths: &DvsPaths,
    filter: Option<&StatusFilter>,
) -> Result<crate::BatchOutcome<StatusSuccess, StatusError>> {
    let dvs_directory = paths.metadata_folder();
    log::debug!("Scanning metadata folder: {}", dvs_directory.display());
    let cache = try_open_cache(paths);

    let entries: Vec<PathBuf> = WalkDir::new(&dvs_directory)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|ext| ext == "dvs").unwrap_or(false))
        .map(|e| e.into_path())
        .collect();

    let pool = get_threadpool(entries.len())?;

    enum Row { Ok(StatusSuccess), Err(StatusError), Skip }

    let mut rows: Vec<Row> = pool.install(|| {
        entries.into_par_iter().map(|dvs_path| {
            let relative = match dvs_path.strip_prefix(&dvs_directory) {
                Ok(r) => r.with_extension(""),
                Err(e) => return Row::Err(StatusError::RelativePath {
                    metadata_path: dvs_path.clone(),
                    reason: e.to_string(),
                }),
            };
            if let Some(f) = filter { if !f.matches(&relative) { return Row::Skip; } }
            match get_file_status(paths, &relative, cache.as_ref()) {
                Ok((status, metadata)) => Row::Ok(StatusSuccess {
                    path: relative,
                    status,
                    metadata,
                }),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("blake3") || msg.contains("hash") {
                        Row::Err(StatusError::HashFailure { path: relative, reason: msg })
                    } else {
                        Row::Err(StatusError::MetadataRead { path: relative, reason: msg })
                    }
                }
            }
        }).collect()
    });

    let mut outcome = crate::BatchOutcome::<StatusSuccess, StatusError>::new();
    rows.sort_by(|a, b| {
        fn key<'a>(r: &'a Row) -> &'a std::path::Path {
            match r {
                Row::Ok(s)  => s.path.as_path(),
                Row::Err(StatusError::RelativePath { metadata_path, .. }) => metadata_path.as_path(),
                Row::Err(StatusError::MetadataRead { path, .. }) => path.as_path(),
                Row::Err(StatusError::HashFailure { path, .. }) => path.as_path(),
                Row::Skip => std::path::Path::new(""),
            }
        }
        key(a).cmp(key(b))
    });
    for row in rows {
        match row {
            Row::Ok(s)  => outcome.ok.push(s),
            Row::Err(e) => outcome.err.push(e),
            Row::Skip   => {}
        }
    }

    log::debug!("Found {} tracked files ({} errors)", outcome.ok.len(), outcome.err.len());
    Ok(outcome)
}
```

- [ ] **Step 8.2: Update existing `status.rs` tests**

Any test that previously asserted on `results[i].detail` must now assert on `outcome.ok[i]` / `outcome.err[i]` with `matches!` on `StatusError` variants. Apply mechanically; no new behavior.

- [ ] **Step 8.3: Update CLI Status branch**

In `dvs-cli/src/main.rs`, replace the `Command::Status { .. }` match arm body (lines ~303–398) with:

```rust
        Command::Status {
            paths: user_paths, recursive, current, absent, unsynced, with_metadata,
        } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let show_all = !current && !absent && !unsynced;

            let filter = if user_paths.is_empty() {
                None
            } else {
                Some(StatusFilter::from_user_paths(user_paths, recursive, &dvs_paths))
            };
            let mut outcome = get_status(&dvs_paths, filter.as_ref())?;
            if !show_all {
                outcome.ok.retain(|s| {
                    (current && s.status == Status::Current)
                        || (absent && s.status == Status::Absent)
                        || (unsynced && s.status == Status::Unsynced)
                });
            }

            if cli.json {
                println!("{}", serde_json::to_string(&outcome)?);
            } else {
                output::print_status_failures(&outcome.err);
                if outcome.ok.is_empty() {
                    println!("{}", if show_all { "No tracked files" } else { "No tracked files matching the filter" });
                } else if with_metadata {
                    let rows: Vec<StatusRowFull> = outcome.ok.iter().map(|s| {
                        let mut row = s.metadata.as_ref().map(StatusRowFull::from).unwrap_or_default();
                        row.path = s.path.display().to_string();
                        row.status = s.status.to_string();
                        row
                    }).collect();
                    println!("{}", tabled::Table::new(rows).to_string());
                } else {
                    let rows: Vec<StatusRow> = outcome.ok.iter().map(|s| StatusRow {
                        path: s.path.display().to_string(),
                        status: s.status.to_string(),
                        size: s.metadata.as_ref().map(|m| format_size(m.size)).unwrap_or_default(),
                    }).collect();
                    println!("{}", tabled::Table::new(rows).to_string());
                }
            }

            if !outcome.err.is_empty() {
                std::process::exit(1);
            }
        }
```

Drop `StatusDetail` from the `use dvs::{…}` import.

- [ ] **Step 8.4: R binding — `dvs_status_impl`**

Replace `dvs_status` in `dvs-rpkg/src/rust/lib.rs` with:

```rust
#[miniextendr(r_name = "dvs_status_impl")]
pub(crate) fn dvs_status(
    #[miniextendr(default = "character(0)")] paths: Vec<PathBuf>,
    #[miniextendr(default = "NULL")] recursive: Option<bool>,
    #[miniextendr(match_arg, several_ok)] status: Vec<StatusChoice>,
) -> Result<List> {
    let current_dir = std::env::current_dir()?;
    let config = Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
    let dvs_paths = DvsPaths::from_cwd(&config)?;

    let show_all = status.is_empty() || status.len() == StatusChoice::CHOICES.len();
    let filter = if paths.is_empty() {
        None
    } else {
        Some(StatusFilter::from_user_paths(paths, recursive.unwrap_or(false), &dvs_paths))
    };
    let mut outcome = get_status(&dvs_paths, filter.as_ref())?;
    if !show_all {
        outcome.ok.retain(|s| status.iter().any(|c| s.status == Status::from(*c)));
    }

    // The existing `FileStatusView` serde view omitted `add_time`; we must
    // keep that behavior for the $ok side so the POSIXct column is appended.
    let views: Vec<StatusSuccessView<'_>> = outcome.ok.iter().map(StatusSuccessView::from).collect();
    let add_times: Vec<Option<OffsetDateTime>> = outcome.ok.iter()
        .map(|s| match &s.metadata {
            Some(m) => OffsetDateTime::from_unix_timestamp_nanos(m.add_time.as_nanosecond()).ok(),
            None => None,
        })
        .collect();
    let add_time_sexp = add_times.into_sexp();

    let ok_df = miniextendr_api::serde::vec_to_dataframe(&views)
        .map_err(|e| anyhow!("serializing status ok rows: {e}"))?
        .drop("metadata_hashes_md5")
        .strip_prefix("metadata_hashes_")
        .strip_prefix("metadata_")
        .rename("blake3", "hash")
        .with_column("add_time", add_time_sexp);

    let mut l = list!("ok" = ok_df);
    if !outcome.err.is_empty() {
        let err_df = miniextendr_api::serde::vec_to_dataframe(&outcome.err)
            .map_err(|e| anyhow!("serializing status err rows: {e}"))?;
        l.set("err", err_df)
            .map_err(|e| anyhow!("setting err on list: {e}"))?;
    }
    Ok(l)
}

#[derive(Serialize)]
struct StatusSuccessView<'a> {
    path: &'a Path,
    status: &'a Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<FileMetadataView<'a>>,
}

impl<'a> From<&'a StatusSuccess> for StatusSuccessView<'a> {
    fn from(s: &'a StatusSuccess) -> Self {
        StatusSuccessView {
            path: s.path.as_path(),
            status: &s.status,
            metadata: s.metadata.as_ref().map(FileMetadataView::from),
        }
    }
}
```

Delete the now-unused `FileStatusView` and its `From<&FileStatus>` impl (the `FileMetadataView` is still used by `StatusSuccessView` — keep it).

- [ ] **Step 8.5: Extend `test-status.R`**

In `dvs-rpkg/tests/testthat/test-status.R`, add:

```r
test_that("dvs_status returns $ok only when nothing fails", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "a.csv")
  dvs::dvs_add("a.csv")

  res <- dvs::dvs_status()
  expect_true(!is.null(res$ok))
  expect_null(res$err)
  expect_equal(nrow(res$ok), 1L)
  expect_equal(res$ok$path, "a.csv")
})
```

(Preserve any existing tests; only append.)

- [ ] **Step 8.6: Run all tests**

```bash
cargo test --workspace
just rpkg-test 2>&1 > /tmp/rpkg.log
```

- [ ] **Step 8.7: Commit**

```bash
git add dvs/src/files/status.rs dvs-cli/src/main.rs dvs-rpkg/src/rust/lib.rs dvs-rpkg/tests/testthat/test-status.R dvs-rpkg/R/dvs-commands.R
git commit -m "dvs,dvs-cli,dvs-rpkg: port 'status' to BatchOutcome"
```

---

## Task 9: Cleanup — delete the old types

At this point nothing consumes `AddResult`, `AddDetail`, `GetResult`, `GetDetail`, `FileStatus`, or `StatusDetail`. Remove them.

**Files:** `dvs/src/files/add.rs`, `dvs/src/files/get.rs`, `dvs/src/files/status.rs`, `dvs/src/lib.rs`

- [ ] **Step 9.1: Remove the old Add types**

In `dvs/src/files/add.rs`, delete:

```rust
pub struct AddResult { … }
pub enum   AddDetail { … }
```

Also delete any internal helpers (e.g. `From<AddResult> for …`) that were used only by them.

- [ ] **Step 9.2: Remove the old Get types**

In `dvs/src/files/get.rs`, delete `GetResult` and `GetDetail`.

- [ ] **Step 9.3: Remove the old Status types**

In `dvs/src/files/status.rs`, delete `FileStatus` and `StatusDetail`. The `StatusFilter` struct stays.

- [ ] **Step 9.4: Update `lib.rs` re-exports**

Edit `dvs/src/lib.rs`. The final `pub use` block should read (ordering unchanged from existing convention):

```rust
pub use backends::Backend;
pub use batch::BatchOutcome;
pub use config::Compression;
pub use files::add::{AddError, AddSuccess, add_files};
pub use files::get::{GetError, GetSuccess, get_files};
pub use files::metadata::FileMetadata;
pub use files::status::{StatusError, StatusFilter, StatusSuccess, get_status, FileStatus /* keep if used elsewhere */};
pub use files::types::{Outcome, Status};
pub use hashes::{HashAlg, Hashes};
pub use paths::{AddPathStatus, DvsPaths, find_repo_root};
pub use progress::FileProgress;
pub use utils::{format_size, set_num_threads};
```

> **Implementer:** if `FileStatus` was deleted in 9.3, remove it from this list. Likewise drop any stray `*Detail` / `*Result` that might still appear.

- [ ] **Step 9.5: Final sweep — confirm no dead references**

Run:

```bash
rg "AddResult|AddDetail|GetResult|GetDetail|FileStatus|StatusDetail" dvs dvs-cli dvs-rpkg || echo OK
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
just rpkg-test 2>&1 > /tmp/rpkg.log
```

Expected: `rg` prints `OK` (no references anywhere). All builds / tests / clippy green.

- [ ] **Step 9.6: Commit**

```bash
git add -A
git commit -m "dvs: remove legacy AddResult/GetResult/FileStatus types

All callers migrated to BatchOutcome<*Success, *Error>. The old
*Result/*Detail shapes are no longer referenced anywhere in the
workspace — deleting per spec migration plan, step 4."
```

---

## Post-plan verification

After Task 9, open the PR off `design/batch-outcome-type` against `origin/main` and verify:

- CI green.
- `dvs add` with a mix of valid, nonexistent, outside-project, and directory inputs prints the errors-first grouped block and exits `1`.
- `dvs add .` in a clean repo with no matches still prints a reasonable message.
- `dvs status` with a corrupted `.dvs` entry surfaces `status_error` grouped and keeps the table for the rest.
- R: `dvs::dvs_add(c("ok.csv", "missing.csv"))$err$kind` returns a character vector.
- R: `dvs::dvs_add("ok.csv")$err` is `NULL` when no failures.
- `dvs --json add ...` emits a single JSON object with `ok` and `err` keys; parse-roundtrip with `jq`.
