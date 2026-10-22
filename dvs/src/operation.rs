//! Operation context (observer hooks, cancel flag), counts, byte totals and
//! timing helpers.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::files::add::AddResult;
use crate::files::get::GetRow;
use crate::files::status::StatusFacts;
use crate::progress::FileProgress;

/// A shared "stop starting new files" flag. Cloning shares the flag, so a
/// signal handler can hold one clone while the operation checks another.
///
/// Cancelling never interrupts a file already in flight: it finishes
/// atomically, and only files that had not started are reported `cancelled`.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// How one file of an add or status run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Succeeded,
    Failed,
    /// A cancel stopped it before it started
    Cancelled,
}

/// A result row that can be counted into [`Counts`].
pub trait OperationRow {
    fn state(&self) -> RowState;
}

/// How an add or status run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Complete,
    Partial,
    Failed,
    /// A cancel came before the run finished, even if every file had started
    Cancelled,
}

impl RunOutcome {
    /// `cancelled`: the cancel flag was set when the run finished.
    pub fn of(counts: &Counts, cancelled: bool) -> Self {
        if cancelled {
            RunOutcome::Cancelled
        } else if counts.failed == 0 {
            RunOutcome::Complete
        } else if counts.failed == counts.total {
            RunOutcome::Failed
        } else {
            RunOutcome::Partial
        }
    }
}

/// A stage of an operation. A phase that does not occur is not reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Selecting and validating the files to work on
    Scan,
    /// Hashing working files
    Hash,
    /// Asking storage what it has (access check, manifest, existence checks)
    CheckStorage,
    /// Moving bytes to or from storage
    Transfer,
    /// Writing records, baselines and the gitignore
    Record,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Scan => "scan",
            Phase::Hash => "hash",
            Phase::CheckStorage => "check_storage",
            Phase::Transfer => "transfer",
            Phase::Record => "record",
        }
    }
}

/// What an operation is about to do, known once selection and storage checks
/// are done. `status` reports zero `*_to_transfer`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct OperationPlan {
    pub files_total: u64,
    pub bytes_total: u64,
    pub files_to_transfer: u64,
    pub bytes_to_transfer: u64,
}

/// Byte totals for an `add` or `get` result: `to_transfer + already_stored
/// == total`; `transferred` is what actually moved. Rows of unknown size count
/// in none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ByteTotals {
    pub total: u64,
    pub to_transfer: u64,
    pub already_stored: u64,
    pub transferred: u64,
}

impl ByteTotals {
    pub(crate) fn count(&mut self, size: u64, stored: bool, transferred: bool) {
        self.total += size;
        if stored {
            self.already_stored += size;
        } else {
            self.to_transfer += size;
        }
        if transferred {
            self.transferred += size;
        }
    }
}

/// Row counts for a result: `succeeded + failed + cancelled == total`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
}

impl Counts {
    /// For results that cannot be cancelled.
    pub fn new(total: usize, failed: usize) -> Self {
        Counts {
            total,
            succeeded: total - failed,
            failed,
            cancelled: 0,
        }
    }

    pub fn of<'a, R: OperationRow + 'a>(rows: impl IntoIterator<Item = &'a R>) -> Self {
        let mut counts = Counts::default();
        for row in rows {
            counts.total += 1;
            match row.state() {
                RowState::Succeeded => counts.succeeded += 1,
                RowState::Failed => counts.failed += 1,
                RowState::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }
}

/// A file's final row, in the same shape the command's result will hold.
#[derive(Debug, Clone, Copy)]
pub enum FileRow<'a> {
    Add(&'a AddResult),
    Get(&'a GetRow),
    Status(&'a StatusFacts),
}

/// Hooks an operation calls as it runs. Called from worker threads, possibly
/// concurrently; implementations serialize their own output.
pub trait OperationObserver: Send + Sync {
    /// Once, after selection and storage checks, before any transfer.
    fn planned(&self, _plan: &OperationPlan) {}

    fn phase_started(&self, _phase: Phase) {}

    fn phase_finished(&self, _phase: Phase) {}

    /// A transfer of `total_bytes` for `path` is starting. The returned
    /// callbacks receive byte deltas and the transfer's success; `None` means
    /// nobody is watching this transfer.
    fn transfer_started(&self, _path: &Path, _total_bytes: u64) -> Option<FileProgress> {
        None
    }

    /// A file's outcome is final, in completion order.
    fn file_finished(&self, _row: FileRow<'_>) {}
}

/// A started phase that reports `phase_finished` when dropped, so a phase is
/// closed on every exit, `?` and `bail!` included.
#[must_use = "dropping the guard finishes the phase at once"]
pub(crate) struct PhaseGuard<'a> {
    observer: &'a dyn OperationObserver,
    phase: Phase,
}

impl<'a> PhaseGuard<'a> {
    /// Reports `phase_started` now and `phase_finished` on drop.
    pub(crate) fn start(observer: &'a dyn OperationObserver, phase: Phase) -> Self {
        observer.phase_started(phase);
        Self { observer, phase }
    }

    /// Finishes the phase here, for readability; the same as dropping it.
    pub(crate) fn finish(self) {}
}

impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        self.observer.phase_finished(self.phase);
    }
}

/// A phase that starts the first time work in it begins, from any thread, and
/// is never reported if no work does. Finished on drop if it started.
pub(crate) struct LazyPhase<'a> {
    observer: &'a dyn OperationObserver,
    phase: Phase,
    started: std::sync::Once,
}

impl<'a> LazyPhase<'a> {
    pub(crate) fn new(observer: &'a dyn OperationObserver, phase: Phase) -> Self {
        Self {
            observer,
            phase,
            started: std::sync::Once::new(),
        }
    }

    /// Call before each piece of work in the phase. The first call reports
    /// `phase_started`; concurrent callers wait for it, so no work in the
    /// phase is reported before the phase is.
    pub(crate) fn begin(&self) {
        self.started
            .call_once(|| self.observer.phase_started(self.phase));
    }

    /// Finishes the phase here if it started; the same as dropping it.
    pub(crate) fn finish(self) {}
}

impl Drop for LazyPhase<'_> {
    fn drop(&mut self) {
        if self.started.is_completed() {
            self.observer.phase_finished(self.phase);
        }
    }
}

/// Hears nothing: what an unobserved operation reports to.
struct NoopObserver;

impl OperationObserver for NoopObserver {}

/// Adapts an `on_file_start` callback (the pre-observer progress API) into an
/// observer that only watches transfers.
pub struct TransferObserver<F>(pub F);

impl<F> OperationObserver for TransferObserver<F>
where
    F: Fn(&Path, u64) -> FileProgress + Send + Sync,
{
    fn transfer_started(&self, path: &Path, total_bytes: u64) -> Option<FileProgress> {
        Some((self.0)(path, total_bytes))
    }
}

/// Passed to every operation: who, if anyone, observes it, and its cancel flag.
#[derive(Clone, Default)]
pub struct OperationContext {
    observer: Option<Arc<dyn OperationObserver>>,
    cancel: CancelToken,
}

impl OperationContext {
    /// Observed by nobody, never cancelled unless asked.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Check before starting each file.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn with_observer(mut self, observer: Arc<dyn OperationObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub(crate) fn observer(&self) -> &dyn OperationObserver {
        self.observer.as_deref().unwrap_or(&NoopObserver)
    }

    /// Whether anyone hears the reports, so work done only for them can be skipped.
    pub(crate) fn is_observed(&self) -> bool {
        self.observer.is_some()
    }
}

pub fn duration_ms(since: Instant) -> u64 {
    millis(since.elapsed())
}

pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Unix milliseconds, for event `ts`.
pub fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// RFC 3339 UTC with milliseconds, e.g. `2025-10-21T14:30:00.123Z`.
pub fn rfc3339_ms(time: SystemTime) -> String {
    match jiff::Timestamp::try_from(time) {
        Ok(ts) => format!("{ts:.3}"),
        Err(_) => format!("{:.3}", jiff::Timestamp::UNIX_EPOCH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row(RowState);
    impl OperationRow for Row {
        fn state(&self) -> RowState {
            self.0
        }
    }

    #[test]
    fn counts_split_states_and_a_cancel_decides_the_outcome() {
        use RowState::*;
        let counts = Counts::of(&[Row(Succeeded), Row(Failed), Row(Cancelled)]);
        assert_eq!(
            counts,
            Counts {
                total: 3,
                succeeded: 1,
                failed: 1,
                cancelled: 1
            }
        );
        // Cancelled even when every file had already started
        let started = Counts::of(&[Row(Succeeded)]);
        assert_eq!(RunOutcome::of(&started, true), RunOutcome::Cancelled);
        assert_eq!(RunOutcome::of(&started, false), RunOutcome::Complete);
        let outcome = |rows: &[Row]| RunOutcome::of(&Counts::of(rows), false);
        assert_eq!(outcome(&[Row(Failed)]), RunOutcome::Failed);
        assert_eq!(outcome(&[Row(Succeeded), Row(Failed)]), RunOutcome::Partial);
        assert_eq!(outcome(&[]), RunOutcome::Complete);
    }

    #[test]
    fn a_started_phase_finishes_on_an_early_error_exit() {
        let recorder = crate::testutil::Recorder::default();
        let early_exit = || -> Result<(), ()> {
            let _scan = PhaseGuard::start(&recorder, Phase::Scan);
            Err(())?;
            Ok(())
        };
        assert!(early_exit().is_err());
        assert_eq!(recorder.seen(), ["start scan", "end scan"]);
    }
}
