//! The CLI's one [`OperationObserver`]: what an `add`, `get` or `status`
//! reports while it runs becomes NDJSON events (`--emit-events`) or human
//! progress bars (a terminal), never both.

use std::path::Path;

use dvs::{FileProgress, FileRow, OperationObserver, OperationPlan, Phase};
use serde_json::Value;

use crate::events::{self, EventStream};

type Transfers = Box<dyn Fn(&Path, u64) -> FileProgress + Send + Sync>;
type Rows = Box<dyn Fn(FileRow<'_>) -> Option<Value> + Send + Sync>;

pub struct CliObserver {
    events: Option<&'static EventStream>,
    /// Human progress bars for transfers; unused with events
    bars: Option<Transfers>,
    /// Turns a finished row into its `file_result`, the same shape the
    /// final result holds; `None` leaves the row out of the stream (status
    /// rows a filter does not select)
    rows: Option<Rows>,
    /// Dry runs move nothing, so they report no transfers
    transfers: bool,
}

impl CliObserver {
    /// Events when they are active, else nothing: add human bars with
    /// [`Self::with_bars`].
    pub fn new(dry_run: bool) -> Self {
        Self {
            events: events::active(),
            bars: None,
            rows: None,
            transfers: !dry_run,
        }
    }

    /// Show human progress bars for transfers (only without events).
    pub fn with_bars(
        mut self,
        bars: impl Fn(&Path, u64) -> FileProgress + Send + Sync + 'static,
    ) -> Self {
        if self.events.is_none() {
            self.bars = Some(Box::new(bars));
        }
        self
    }

    /// Stream each finished row as a `file_result`, rendered by `row`.
    pub fn with_rows(
        mut self,
        row: impl Fn(FileRow<'_>) -> Option<Value> + Send + Sync + 'static,
    ) -> Self {
        self.rows = Some(Box::new(row));
        self
    }
}

impl OperationObserver for CliObserver {
    fn planned(&self, plan: &OperationPlan) {
        if let Some(events) = self.events {
            events.planned(plan);
        }
    }

    fn phase_started(&self, phase: Phase) {
        if let Some(events) = self.events {
            events.phase_started(phase);
        }
    }

    fn phase_finished(&self, phase: Phase) {
        if let Some(events) = self.events {
            events.phase_finished(phase);
        }
    }

    fn transfer_started(&self, path: &Path, total_bytes: u64) -> Option<FileProgress> {
        if !self.transfers {
            return None;
        }
        match (self.events, &self.bars) {
            (Some(events), _) => Some(events.progress(path, total_bytes)),
            (None, Some(bars)) => Some(bars(path, total_bytes)),
            (None, None) => None,
        }
    }

    fn file_finished(&self, row: FileRow<'_>) {
        let Some(events) = self.events else {
            return;
        };
        if let Some(value) = self.rows.as_ref().and_then(|render| render(row)) {
            events.file_result(value);
        }
        events.file_done();
    }
}
