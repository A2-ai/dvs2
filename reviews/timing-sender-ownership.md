# Timing CSV deadlock: Sender ownership in OutputOptions

## Date

2026-03-11

## Bug

`dvs add -vvv` produced an empty timing CSV file. The program appeared to complete normally but the CSV was never flushed.

## Root cause

`OutputOptions` embeds an `Option<Sender<TimingRecord>>`:

```rust
pub struct OutputOptions {
    pub dry_run: bool,
    pub verbosity: u8,
    pub timing_tx: Option<Sender<TimingRecord>>,
}
```

In `make_output()`, the `TimingHandle` creates a `mpsc::channel`. The handle keeps one `Sender`, and a clone is placed into `OutputOptions`. The receiver thread runs `for record in rx { ... }` — this loop only exits when **all** senders are dropped.

The CLI code was structured as:

```rust
let (output, timing_handle) = make_output(verbosity, dry_run);
let add_opts = AddOptions { output, ... };
let results = add_files(..., &add_opts)?;
if let Some(h) = timing_handle {
    h.finish();  // drops handle's Sender, calls join() on receiver thread
}
// add_opts still alive here — its cloned Sender keeps the channel open
```

`TimingHandle::finish()` drops its own sender and calls `thread::JoinHandle::join()`. But `add_opts.output.timing_tx` still holds a live `Sender` clone. The receiver thread blocks waiting for more records because it sees an active sender. `join()` blocks forever. The CSV writer's `flush()` (which runs after the `for` loop) is never reached.

The program appeared to exit because in some contexts (background task runners, timeouts) the process was killed externally before the hang was noticed. In an interactive terminal, the program would hang indefinitely after printing "Done in ...".

The same bug existed in all three command branches: `Add`, `Status`, and `Get`.

## Fix applied

```rust
let results = add_files(..., &add_opts)?;
drop(add_opts);  // release the cloned Sender
if let Some(h) = timing_handle {
    h.finish();  // now join() completes because all Senders are gone
}
```

Applied to all three command branches. Also fixed `TimingHandle::shutdown()` printing "Timing log saved" twice (once from `finish()`, once from `Drop`).

## Structural concern

The fix is correct and minimal, but the design has a non-obvious ownership coupling: `OutputOptions` holds a value (`Sender`) whose lifetime is entangled with an external resource (`TimingHandle`'s receiver thread). Nothing in the type system prevents the deadlock — it relies on the caller dropping `OutputOptions` at the right time relative to `TimingHandle::finish()`.

A structurally sounder design would separate the timing transport from the output options:

### Option A: Remove Sender from OutputOptions

`OutputOptions` holds only `dry_run` and `verbosity`. The `Sender` (or an `&TimingHandle`) is passed as a separate parameter to functions that emit timing records.

**Pros**: clear ownership, no hidden lifetime coupling, impossible to deadlock.
**Cons**: touches many function signatures (`add_files`, `get_files`, `get_status`, `metadata.save`, `cache::hashes_for_file`, and all their callers). Significant churn.

### Option B: TimingHandle owns the sole Sender, exposes &self method

`TimingHandle` wraps the `Sender` in an `Arc<Sender>` or exposes a `send(&self, record)` method. `OutputOptions` holds an `Arc` clone or a reference to the handle. Dropping the handle closes the channel regardless of how many `Arc` clones exist, because `finish()` would also drop the receiver side or signal shutdown.

**Pros**: fewer signature changes, ownership is clearer.
**Cons**: `Arc` adds indirection; signaling shutdown from the sender side requires additional mechanism (e.g., a separate shutdown flag).

### Option C: Use crossbeam channel

`crossbeam::channel::Sender` is `Clone + Send + Sync`. The channel can be explicitly closed via `drop(sender)` on the handle side without waiting for all clones to drop — or the receiver can be dropped to signal shutdown.

**Pros**: more ergonomic channel API, bounded channels available.
**Cons**: new dependency.

### Decision

The `drop` fix is sufficient. The coupling is localized to three call sites in `main.rs`, each following the same pattern. The comment on the `drop` explains why it's there. A structural refactor would touch ~10 function signatures across `dvs/src/` for a problem that manifests in exactly one place (the CLI entry point). Not worth the churn given the current codebase size.

If `OutputOptions` grows more fields with external ownership semantics, or if the timing transport is needed in more contexts, revisit Option A.
