//! Operation context and observer hooks, counts, byte totals and timing helpers.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::files::add::AddResult;
use crate::files::get::GetRow;
use crate::inspect::StatusFacts;
use crate::progress::FileProgress;

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

/// Row counts for a result: `succeeded + failed == total`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
}

impl Counts {
    pub fn new(total: usize, failed: usize) -> Self {
        Counts {
            total,
            succeeded: total - failed,
            failed,
        }
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
pub struct PhaseGuard<'a> {
    observer: &'a dyn OperationObserver,
    phase: Phase,
}

impl<'a> PhaseGuard<'a> {
    /// Reports `phase_started` now and `phase_finished` on drop.
    pub fn start(observer: &'a dyn OperationObserver, phase: Phase) -> Self {
        observer.phase_started(phase);
        Self { observer, phase }
    }

    /// Finishes the phase here, for readability; the same as dropping it.
    pub fn finish(self) {}
}

impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        self.observer.phase_finished(self.phase);
    }
}

/// A phase that starts the first time work in it begins, from any thread, and
/// is never reported if no work does. Finished on drop if it started.
pub struct LazyPhase<'a> {
    observer: &'a dyn OperationObserver,
    phase: Phase,
    started: std::sync::Once,
}

impl<'a> LazyPhase<'a> {
    pub fn new(observer: &'a dyn OperationObserver, phase: Phase) -> Self {
        Self {
            observer,
            phase,
            started: std::sync::Once::new(),
        }
    }

    /// Call before each piece of work in the phase. The first call reports
    /// `phase_started`; concurrent callers wait for it, so no work in the
    /// phase is reported before the phase is.
    pub fn begin(&self) {
        self.started
            .call_once(|| self.observer.phase_started(self.phase));
    }

    /// Finishes the phase here if it started; the same as dropping it.
    pub fn finish(self) {}
}

impl Drop for LazyPhase<'_> {
    fn drop(&mut self) {
        if self.started.is_completed() {
            self.observer.phase_finished(self.phase);
        }
    }
}

/// Hears nothing. The default observer.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopObserver;

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

/// Passed to every operation. One context is one operation: one start time.
#[derive(Clone)]
pub struct OperationContext {
    started: Instant,
    started_at: SystemTime,
    observer: Arc<dyn OperationObserver>,
}

impl Default for OperationContext {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationContext {
    /// Starts now, observed by nobody.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            started_at: SystemTime::now(),
            observer: Arc::new(NoopObserver),
        }
    }

    pub fn with_observer(mut self, observer: Arc<dyn OperationObserver>) -> Self {
        self.observer = observer;
        self
    }

    pub fn observer(&self) -> &dyn OperationObserver {
        &*self.observer
    }

    /// Monotonic start, for `elapsed_ms` and durations.
    pub fn started(&self) -> Instant {
        self.started
    }

    /// Wall-clock start, for `started_at`.
    pub fn started_at(&self) -> SystemTime {
        self.started_at
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

    #[derive(Default)]
    struct Phases(std::sync::Mutex<Vec<(bool, Phase)>>);

    impl OperationObserver for Phases {
        fn phase_started(&self, phase: Phase) {
            self.0.lock().unwrap().push((true, phase));
        }
        fn phase_finished(&self, phase: Phase) {
            self.0.lock().unwrap().push((false, phase));
        }
    }

    #[test]
    fn phase_guards_finish_on_every_exit() {
        let seen = Phases::default();
        let early_exit = || -> Result<(), ()> {
            let _scan = PhaseGuard::start(&seen, Phase::Scan);
            Err(())?;
            Ok(())
        };
        assert!(early_exit().is_err());
        // A lazy phase nobody began is never reported; one begun twice is
        // reported once
        LazyPhase::new(&seen, Phase::CheckStorage).finish();
        let transfer = LazyPhase::new(&seen, Phase::Transfer);
        transfer.begin();
        transfer.begin();
        drop(transfer);
        assert_eq!(
            *seen.0.lock().unwrap(),
            vec![
                (true, Phase::Scan),
                (false, Phase::Scan),
                (true, Phase::Transfer),
                (false, Phase::Transfer),
            ]
        );
    }
}
