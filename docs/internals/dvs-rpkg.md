# `dvs-rpkg` internals

The `dvs-rpkg` R package lives at `/dvs-rpkg/`. It wraps the `dvs` core crate over a `miniextendr` FFI bridge.

## 1. Architecture overview

The package is organized into three distinct layers. At the top, the R public API lives in `R/dvs-commands.R`. These are hand-written, exported functions (`dvs_init`, `dvs_add`, `dvs_status`, `dvs_get`) that add option sync, progress-token creation, and tibble post-processing on top of the generated bindings. Beneath them sit the auto-generated R bindings in `R/dvs-wrappers.R`. Those are thin `.Call()` shims targeting C entry points registered by the cdylib; the file is regenerated wholesale on every install, so it should never be edited by hand. At the bottom is the Rust FFI crate at `src/rust/lib.rs`. It contains the `#[miniextendr]`-exported functions, which in turn call into the standalone `dvs` core crate (init, add, get, status, hashing, storage I/O).

The file `dvs-rpkg/src/rust/Cargo.toml` declares `[workspace]` with no `members` key, which makes it a standalone workspace completely separate from the root workspace. In release builds `dvs` is fetched from git, and in monorepo development a `[patch]` block (uncommitted) or the `just vendor` workflow redirects the dependency to `../../../dvs`.

A few build mechanics are worth knowing. During `R CMD INSTALL`, the FFI crate is first compiled as a cdylib and loaded into R so that `miniextendr_write_wrappers()` can emit `R/dvs-wrappers.R`. That cdylib is then discarded. The final `dvs.so` or `dvs.dll` is linked from a staticlib build of the same crate, combined with a thin C stub at `src/stub.c`. Function self-registration uses `linkme::distributed_slice`, so there is no manual registry table to maintain.

## 2. FFI boundary

### Exported functions (`src/rust/lib.rs`)

| Rust fn | R-visible name | Args (Rust types) | Return type |
|---|---|---|---|
| `dvs_init` | `dvs_init_impl` | `storage_path: PathBuf`, `root_dir: Option<PathBuf>`, `group: Option<String>`, `metadata_folder_name: Option<String>`, `compression: CompressionChoice` | `Result<List>` |
| `dvs_add` | `dvs_add_impl` | `paths: Vec<PathBuf>`, `message: Option<String>`, `glob: Option<String>`, `dry_run: Option<bool>`, `progress_callback: Option<ExternalPtr<ProgressBarCallback>>` | `Result<List>` |
| `dvs_status` | `dvs_status_impl` | `paths: Vec<PathBuf>`, `recursive: Option<bool>`, `status: Vec<StatusChoice>` | `Result<List>` |
| `dvs_get` | `dvs_get_impl` | `paths: Vec<PathBuf>`, `glob: Option<String>`, `dry_run: Option<bool>`, `progress_callback: Option<ExternalPtr<ProgressBarCallback>>` | `Result<List>` |
| `dvs_set_threads` | `dvs_set_threads_impl` | `threads: Option<usize>` | `()` (invisible NULL) |
| `format_byte_size` | `format_byte_size` | `size_bytes: u64` | `String` |
| `dvs_version` | `dvs_version` | (none) | `String` |
| `set_dvs_log_level` | `set_dvs_log_level` | `level: log::LevelFilter` (via `match_arg`) | `()` (invisible NULL) |
| `ProgressBarCallback::new` | `ProgressBarCallback$new()` | (none) | `ExternalPtr<ProgressBarCallback>` (class `"ProgressBarCallback"`) |
| `dvs_test_2_5_gib_bytes` | `dvs_test_2_5_gib_bytes` | (none) | `u64` (noexport, test-only) |

On visibility and naming: `dvs_init_impl` and `set_dvs_log_level` return invisibly. `dvs_add_impl`, `dvs_status_impl`, `dvs_get_impl`, and `dvs_set_threads_impl` are `pub(crate)` with explicit `r_name` attributes, which keeps their R-visible names distinct from their Rust identifiers.

Two enum types participate in argument marshalling via the `MatchArg` derive macro. `CompressionChoice { Zstd, None }` maps to the R character vector `c("zstd", "none")` and `match.arg()` is enforced on both sides. `StatusChoice { Current, Absent, Unsynced }` maps to `c("current", "absent", "unsynced")` with `several.ok = TRUE` so the R wrapper accepts a vector.

### Type marshalling

Strings (`String`, `Option<String>`) are passed as R `character(1)` or `NULL`. The generated wrapper includes `stopifnot` guards that enforce type and length-1 invariants before `.Call()`.

Paths (`PathBuf`, `Vec<PathBuf>`) are passed as R character vectors that miniextendr-api converts to `PathBuf` per element. No UTF-8 normalization or encoding coercion is performed: if R passes a non-UTF-8 path, the conversion will silently fail or produce a corrupted path.

Booleans (`Option<bool>`) are R `logical(1)` or `NULL`. Unsigned integers (`usize`, `u64`) are R numeric (double); the generated wrapper accepts `is.numeric || is.logical || is.raw` and verifies the value is non-negative.

`ExternalPtr<ProgressBarCallback>` wraps the (zero-sized) Rust struct in miniextendr's `ExternalPtr`. It is returned to R as a classed object of class `"ProgressBarCallback"` and passed back to the FFI unchanged. Its sole purpose is to act as a capability token: its presence in the call signals that a progress bar should be shown, and the struct itself carries no data.

Data frames are produced via `vec_to_dataframe` and `vec_to_dataframe_split` (from miniextendr-api with the serde feature). Each Rust result struct is `#[derive(Serialize)]`, and the serde serializer drives the columnar builder.

For lists, `dvs_init` returns `list!("status" = "initialized")`. The mixed-variant case from `dvs_add`, `dvs_get`, and `dvs_status` returns a named `List` of the form `list(Success = <df>, Error = <df>)`.

### Commit `3e882d2`: event state as data.frame or list-of-data.frames

Before this commit, all three operation functions returned a single flat `ColumnarDataFrame` and used `#[serde(untagged)]` to merge Success and Error rows into one schema padded with `NA` columns. After the commit, the design changed as follows.

Three per-command view enums were introduced: `AddResultView`, `GetResultView`, and the pair `FileStatusSuccessView` and `FileStatusErrorView`. All are externally tagged (of the form `enum { Success { ... }, Error { ... } }`), which lets `vec_to_dataframe_split` partition rows by variant name. The `vec_to_dataframe_split` helper produces a bare data.frame when only one variant appears and a named `list(Success = <df>, Error = <df>)` when both appear.

For `dvs_status`, the partition is done manually rather than via `vec_to_dataframe_split`, so that `add_time` can be injected as a `POSIXct` column directly from `jiff::Timestamp` to `miniextendr_api::time::OffsetDateTime` to R POSIXct, bypassing serde's RFC 3339 string path entirely.

For `dvs_add` and `dvs_get`, when no glob is supplied, paths are fed directly to `add_files` or `get_files` without going through `resolve_paths_for_add` or `resolve_paths_for_get`. That is safe because the per-file validation machinery (`validate_for_add`, `validate_for_get`) already generates `AddDetail::Error` or `GetDetail::Error` rows for invalid inputs.

The R-side `dvs-commands.R` was updated to use a `.dvs_finalize` helper that recursively applies `new_dvs_bytes` to byte columns and `tibble::as_tibble`, preserving whichever shape (bare data.frame or named list) Rust returned.

### Progress bridge (`cli_progress.rs`)

`cli_progress.rs` provides `CliProgressBar`, which wraps R's `cli` package progress bar via thin C shims (`cli_progress_bar_shim`, `cli_progress_add_shim`, and so on) declared as `unsafe extern "C"`. These shims are compiled as part of the R package's C code and linked into `dvs.so`. They call `cli`'s static-inline C API, which is why DESCRIPTION lists `LinkingTo: cli` and `dvs-package.R` has `@importFrom cli cli_progress_bar`. Both serve to force cli's DLL to load before the shims try to resolve `cli`'s callables via `R_GetCCallable`.

Coordination between the worker thread and R's main thread (which owns the progress bar) uses `miniextendr_api::pump::WorkerPump`. The worker sends `ProgressBytes` values over a `std::sync::mpsc::SyncSender<i64>` channel with capacity 64. By convention, a negative value means "file started" (the absolute value is the file size), and a positive value means "bytes transferred." Byte updates are batched: an `AtomicI64` accumulates pending bytes until the count exceeds `BATCH_THRESHOLD` (256 KiB), at which point the entire accumulated value is swapped to zero and sent. `WorkerPump` calls the progress callback on every pump tick from R's main thread, and because `drain_logs_each_tick(true)` is the default, it also drains the cross-thread log queue so Rust-side `log::*!` calls surface in real time.

For `cfg(test)` builds, all five shim functions have no-op stubs, so the FFI crate's unit tests compile and run without an R runtime.

## 3. R-side wrapper layer (`R/dvs-commands.R`)

`R/dvs-commands.R` is the user-facing public API layer. It wraps the auto-generated `*_impl` functions, adding three responsibilities: syncing the thread-count option, managing the `ProgressBarCallback` token, and post-processing the result (applying `new_dvs_bytes` and converting to tibble).

| Public R name | What it does | Calls into |
|---|---|---|
| `dvs_init(storage_path, root_dir, group, metadata_folder_name, compression)` | Initializes a DVS repo | `dvs_init_impl()` |
| `dvs_add(paths, message, glob, dry_run)` | Hashes and copies files into content-addressable storage; shows a progress bar unless `dry_run = TRUE` | `dvs_set_threads_impl()`, `ProgressBarCallback$new()`, `dvs_add_impl()` |
| `dvs_status(paths, recursive, status)` | Reports sync state of tracked files | `dvs_set_threads_impl()`, `dvs_status_impl()` |
| `dvs_get(paths, glob, dry_run)` | Retrieves files from storage into the working directory; shows a progress bar unless `dry_run = TRUE` | `dvs_set_threads_impl()`, `ProgressBarCallback$new()`, `dvs_get_impl()` |

In `dvs_add` and `dvs_get`, the `progress_callback` argument is set to `ProgressBarCallback$new()` whenever `dry_run` is not `TRUE`, and is passed through to the impl. On the Rust side, `if progress_callback.is_some()` is used to decide whether to use `run_with_progress`.

The post-processing rules are simple. `dvs_add` promotes `result$size` and `result$stored_size` to the `dvs_bytes` class. `dvs_status` promotes `status_data_frame$size`. `dvs_get` promotes `get_data_frame$size`. All three call `tibble::as_tibble()` on the result. As of commit `3e882d2`, that logic is expressed via a `.dvs_finalize` helper that handles both the bare data.frame and the `list(Success=, Error=)` shape.

`dvs_init` is a thin pass-through. It calls `match.arg(compression)` and delegates, returning the result invisibly.

## 4. R-side helpers

### `R/dvs-options.R`

This file exports a single function, `set_dvs_threads(threads)`. The thread count is stored as the R option `"dvs.num_threads"` via `options(dvs.num_threads = threads)`. This is standard R `options()` state, not a package-level environment or reference object, so it persists for the session, survives `devtools::load_all()` reloads, and can be set in `.Rprofile` or overridden temporarily with `withr::with_options()`. The function validates that `threads` is a single positive numeric (or `NULL`) and returns the previous value invisibly. The actual sync to the Rust backend happens at call time inside `dvs_add`, `dvs_status`, and `dvs_get`, each of which calls `dvs_set_threads_impl(getOption("dvs.num_threads"))`.

### `R/pillar.R`

This file defines the `dvs_bytes` S3 class, a thin wrapper over R `double` (not `integer`, because R integers are 32-bit and overflow at about 2 GB). The exported functions are as follows. `new_dvs_bytes(x)` is the constructor; it calls `structure(as.double(x), class = c("dvs_bytes", "numeric"))`. `pillar_shaft.dvs_bytes` formats values by calling `format_byte_size()` (the Rust FFI function) per element, producing right-aligned human-readable strings (such as `"1.2 MiB"`) for tibble output. `NA` values pass through as `NA_character_`. `type_sum.dvs_bytes` returns `"bytes"` for the column-type header in tibble display. `Ops.dvs_bytes` preserves the `dvs_bytes` class for `+` and `-`, but strips it for multiplicative and comparison operators, since the result is no longer dimensionally bytes. Finally, `Summary.dvs_bytes` preserves the class for `sum`, `min`, `max`, and `range`, but strips it for `any`, `all`, and `prod`.

For `NA` handling, `pillar_shaft.dvs_bytes` explicitly checks `is.na(v)` per element and returns `NA_character_`. As a result, `NA` byte values display as `NA` rather than propagating an error into `format_byte_size`, which expects a non-negative `u64`.

### `R/dvs-commands.R` and `R/dvs-wrappers.R`

Section 3 covers the role of `dvs-commands.R` in full. The difference from `dvs-wrappers.R` is that `dvs-commands.R` is hand-written, exported, and forms the stable public API, whereas `dvs-wrappers.R` is auto-generated, marked `@keywords internal`, and mostly not exported. The exceptions are `dvs_version`, `format_byte_size`, and `set_dvs_log_level`, which are utility and diagnostic functions exported directly from the generated wrapper. The separation lets the public functions add logic (option sync, progress token creation, tibble conversion) without leaking those concerns into the generated scaffolding.

### `R/dvs-package.R`

This file contains exactly two functional lines beyond the `"_PACKAGE"` roxygen stub. The first, `#' @useDynLib dvs, .registration = TRUE`, causes R to load `dvs.so` and register its C entry points at package load time. The second, `#' @importFrom cli cli_progress_bar`, forces cli's namespace and DLL to load at package load. This is required because `cli_progress.rs` calls cli's C callables via `R_GetCCallable`; if cli's DLL has not loaded by the time the shim tries to resolve those callables, the resolution would fail. Importing a specific cli symbol guarantees that cli is initialized before any DVS operation runs.

## 5. Build and scaffolding

The build uses autoconf (`configure.ac`, about 19 KB) plus a `Makevars.in` template at `src/Makevars.in`. Running `./configure` (or `just rpkg-configure`) generates `src/Makevars` and `src/rust/.cargo/config.toml` from their `.in` templates.

Two install modes are auto-detected by the presence of `inst/vendor.tar.xz`. When the tarball is present (the offline or CRAN path), `configure` unpacks it and writes a vendored cargo source replacement. When it is absent (the dev or monorepo path), Cargo resolves dependencies over the network.

The script `tools/detect-features.R` is called by `configure` to assemble the `CARGO_FEATURES` string. For `dvs-rpkg` it always emits empty unless `DVS_AUTO_FEATURES` is set, because the two optional features (`nonapi` and `connections`) should never be auto-enabled. `bootstrap.R` (triggered via `Config/build/bootstrap: TRUE` in DESCRIPTION) runs `./configure` before `R CMD build` creates a source tarball, which ensures `Makevars` exists in the tarball. Rust dependencies under `vendor/` are generated by `cargo revendor --freeze` (invoked via `just vendor`) and must not be edited by hand.

`crate-type = ["staticlib"]` in `Cargo.toml` means the final link product is a static archive that R's `Makevars` links into `dvs.so`. The cdylib mode used during wrapper generation is a separate compile pass and is not shipped.

## 6. Quirks and gotchas

### Errors crossing the FFI boundary

All `#[miniextendr]`-exported functions that return `Result<T>` have their `Err` converted to an R condition object with class `c("rust_error", "simpleError", "error", "condition")`. The generated wrappers in `dvs-wrappers.R` check `inherits(.val, "rust_error_value") && isTRUE(attr(.val, "__rust_error__"))` immediately after `.Call()` and invoke `.miniextendr_raise_condition()`. That helper dispatches on `kind` (error, warning, message, condition, or panic) and calls `stop()`, `warning()`, `message()`, or `signalCondition()` accordingly. The call object attached to the condition comes from `.call = match.call()` passed to `.Call()`; if a Rust panic payload did not capture a call, the wrapper's `sys.call()` is used as the fallback. The result is that error messages from `anyhow!("...")` propagate cleanly to R's condition system with an accurate call stack entry.

### Ownership and lifetimes

The serde view structs (`AddResultView<'a>`, `GetResultView<'a>`, `FileStatusSuccessView<'a>`, and `FileStatusErrorView<'a>`) borrow from the `Vec<AddResult>`, `Vec<GetResult>`, or `Vec<FileStatus>` owned by the Rust call frame. They are created, serialized into the columnar builder, and dropped entirely within the FFI function body before returning, so no Rust-owned data escapes to R as a borrow. The one live Rust allocation that crosses the boundary is `ProgressBarCallback` wrapped in `ExternalPtr`, which is kept alive by R's GC via `R_PreserveObject`.

### NA handling

The Rust side does not produce `NA` for most scalar fields. `Option<T>` fields serialize as `NA` (via miniextendr's serde integration) when they are `None`. The most important case is `FileStatusSuccessView`'s optional metadata fields (`size`, `hash`, `message`, and so on), which are `None` when a file has metadata but its status is `Absent` (the file is gone from disk). The `add_time` field has special handling: `OffsetDateTime::from_unix_timestamp_nanos(...).ok()` converts a `jiff::Timestamp` to `Option<OffsetDateTime>`, mapping out-of-range timestamps to `None` and therefore to R `NA` in the POSIXct column.

### Thread safety

Rust operations run on a worker thread managed by `WorkerPump`. R's API is not thread-safe, so all R calls (including progress bar updates and log record draining) happen exclusively on R's main thread via the pump's callback mechanism. The `SyncSender<i64>` channel with capacity 64 provides backpressure, so the worker does not get arbitrarily far ahead of the UI thread.

### `RUST_LOG` is silently ignored in R

The R package installs its own logger via miniextendr's `log` feature; it does not call `env_logger::init()`. Setting `RUST_LOG` in the shell before launching R has no effect. The only control is `set_dvs_log_level()`. The default level at load is `"off"`.

### R/Rust name divergence and empty-call behavior

The public R API uses names like `dvs_add(paths=)`, while the Rust function is `dvs_add(paths: Vec<PathBuf>)`. The Rust-side default of `character(0)` for `paths` means R callers can omit the argument, but `dvs_add()` with no arguments will reach Rust with an empty `Vec` and then error with `"No files to add"` rather than producing a more informative R-level error earlier. There is no R-side guard requiring at least one path or glob. The same applies to `dvs_get`.

### Path encoding

Paths are passed as R character strings, converted to Rust `PathBuf` via miniextendr's conversion. No normalization, canonicalization, or encoding conversion is applied at the FFI layer. On macOS, `/tmp` is a symlink to `/private/tmp`, and `std::fs::canonicalize` resolves the symlink. That can cause `starts_with` comparisons to fail if one side was canonicalized and the other wasn't (a known issue in test code, fixed in PR #69 for path comparisons in tests).

### `dvs-wrappers.R` must not be edited by hand

The file header says so, and the `CLAUDE.md` repeats it. The file is regenerated wholesale on every `just rpkg-install`. Any needed change to the R API surface must be made in `src/rust/lib.rs` (doc comments, defaults via `#[miniextendr(default = "...")]`, `r_name`, visibility, or `match_arg` annotations), and the package must then be rebuilt with the full `rpkg-configure`, then `rpkg-install`, then `rpkg-document` sequence.
