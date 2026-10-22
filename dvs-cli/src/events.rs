//! Serialized, flushed NDJSON output for subprocess clients.
//!
//! Transfer callbacks describe transfer work only. The terminal command result
//! remains authoritative: a transfer can finish before installation fails.
//!
//! Every event carries `ts` (unix milliseconds) and `elapsed_ms` (since the
//! operation started). Operation-level progress is aggregated here from
//! the library's facts (plan, transfer byte deltas, finished files) and
//! throttled, so a large transfer emits a bounded number of lines.

use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dvs::operation::{duration_ms, rfc3339_ms, unix_ms};
use dvs::{FileProgress, OperationPlan, Phase};
use serde_json::{Value, json};

static ACTIVE: OnceLock<EventStream> = OnceLock::new();

/// At most one `operation_progress` this often...
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// ...and only once a file finished or this share of the bytes moved.
const PROGRESS_BYTES_PERCENT: u64 = 1;

pub fn active() -> Option<&'static EventStream> {
    ACTIVE.get()
}

pub fn activate(command: &str) -> &'static EventStream {
    ACTIVE.get_or_init(|| EventStream::stdout(command))
}

#[derive(Clone)]
pub struct EventStream {
    state: Arc<Mutex<State>>,
}

struct State {
    writer: Box<dyn Write + Send>,
    operation_id: String,
    seq: u64,
    finished: bool,
    error: Option<io::Error>,
    result: Option<Value>,
    started: Instant,
    started_at: SystemTime,
    progress: Progress,
}

/// The operation's aggregate progress and what the last
/// `operation_progress` said. Nothing is emitted before the plan.
#[derive(Default)]
struct Progress {
    planned: bool,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    /// The plan's `bytes_to_transfer`: what moving bytes count against
    bytes_total: u64,
    /// When the last progress event was emitted, or the plan arrived
    last_at: Option<Instant>,
    last_files: u64,
    last_bytes: u64,
}

impl Progress {
    fn fields(&self) -> Value {
        json!({
            "files_done": self.files_done,
            "files_total": self.files_total,
            "bytes_done": self.bytes_done,
            "bytes_total": self.bytes_total,
        })
    }

    /// Whether the throttle lets a progress event out now.
    fn due(&self, now: Instant) -> bool {
        if !self.planned {
            return false;
        }
        let waited = self
            .last_at
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        let file_finished = self.files_done != self.last_files;
        let moved = self.bytes_done.saturating_sub(self.last_bytes);
        let bytes_moved =
            moved > 0 && moved.saturating_mul(100) >= self.bytes_total * PROGRESS_BYTES_PERCENT;
        waited && (file_finished || bytes_moved)
    }

    /// Whether something changed since the last progress event.
    fn pending(&self) -> bool {
        self.planned && (self.files_done != self.last_files || self.bytes_done != self.last_bytes)
    }

    fn mark(&mut self, now: Instant) {
        self.last_at = Some(now);
        self.last_files = self.files_done;
        self.last_bytes = self.bytes_done;
    }
}

impl EventStream {
    pub fn stdout(command: &str) -> Self {
        Self::new(command, Box::new(io::stdout()))
    }

    fn new(command: &str, writer: Box<dyn Write + Send>) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let clock = crate::output::clock();
        let stream = Self {
            state: Arc::new(Mutex::new(State {
                writer,
                operation_id: format!("{}-{nonce}", std::process::id()),
                seq: 0,
                finished: false,
                error: None,
                result: None,
                started: clock.started,
                started_at: clock.started_at,
                progress: Progress::default(),
            })),
        };
        stream.emit("operation_started", json!({"command": command}), false);
        stream
    }

    fn emit(&self, kind: &str, fields: Value, terminal: bool) {
        let mut state = self.state.lock().unwrap();
        Self::emit_locked(&mut state, kind, fields, terminal);
    }

    /// Write one event with the state already locked, so a caller can update
    /// shared progress and emit from it atomically.
    fn emit_locked(state: &mut State, kind: &str, mut fields: Value, terminal: bool) {
        if state.finished || state.error.is_some() {
            return;
        }
        fields["v"] = json!(1);
        fields["operation_id"] = json!(state.operation_id);
        fields["seq"] = json!(state.seq);
        fields["type"] = json!(kind);
        fields["ts"] = json!(unix_ms(SystemTime::now()));
        fields["elapsed_ms"] = json!(duration_ms(state.started));
        // Serialize before writing so parallel callbacks cannot interleave lines.
        let result = serde_json::to_vec(&fields)
            .map_err(io::Error::other)
            .and_then(|mut line| {
                line.push(b'\n');
                state.writer.write_all(&line)?;
                state.writer.flush()
            });
        state.seq += 1;
        state.finished = terminal;
        if let Err(error) = result {
            state.error = Some(error);
        }
    }

    /// Emit the public authorization instructions, never the secret device or
    /// access token. The caller continues polling through the normal login flow.
    pub fn waiting_for_auth(&self, user_code: &str, approval_url: &str) {
        self.emit(
            "waiting_for_auth",
            json!({"user_code": user_code, "approval_url": approval_url}),
            false,
        );
    }

    pub fn file_result(&self, result: Value) {
        self.emit("file_result", json!({"result": result}), false);
    }

    /// The operation's plan, once, before any file is reported. It also
    /// starts operation progress: files count against `files_total`, moving
    /// bytes against `bytes_to_transfer`.
    pub fn planned(&self, plan: &OperationPlan) {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        let progress = &mut state.progress;
        progress.planned = true;
        progress.files_total = plan.files_total;
        progress.bytes_total = plan.bytes_to_transfer;
        progress.last_at = Some(now);
        Self::emit_locked(&mut state, "operation_planned", json!(plan), false);
    }

    pub fn phase_started(&self, phase: Phase) {
        self.emit("phase_started", json!({"phase": phase.as_str()}), false);
    }

    pub fn phase_finished(&self, phase: Phase) {
        self.emit("phase_finished", json!({"phase": phase.as_str()}), false);
    }

    /// A file's outcome is final: count it, and report progress if due.
    pub fn file_done(&self) {
        self.advance(|progress| progress.files_done += 1);
    }

    fn bytes_moved(&self, delta: u64) {
        self.advance(|progress| progress.bytes_done = progress.bytes_done.saturating_add(delta));
    }

    fn advance(&self, update: impl FnOnce(&mut Progress)) {
        let mut state = self.state.lock().unwrap();
        update(&mut state.progress);
        let now = Instant::now();
        if state.progress.due(now) {
            state.progress.mark(now);
            let fields = state.progress.fields();
            Self::emit_locked(&mut state, "operation_progress", fields, false);
        }
    }

    /// Capture the same result document used by JSON mode. The dispatcher adds
    /// its actual process exit status when it writes the terminal event.
    pub fn record_result(&self, result: Value) {
        if let Some(files) = result.get("files").and_then(Value::as_array) {
            for file in files {
                self.file_result(file.clone());
            }
        }
        self.state.lock().unwrap().result = Some(result);
    }

    /// Capture a result whose file rows were already emitted during the operation.
    pub fn record_streamed_result(&self, result: Value) {
        self.state.lock().unwrap().result = Some(result);
    }

    pub fn take_result(&self) -> Option<Value> {
        self.state.lock().unwrap().result.take()
    }

    /// Write exactly one terminal event: the `--json` document without
    /// `files`, whose rows were already sent as `file_result` events.
    /// Progress the throttle held back is reported first, so the last
    /// `operation_progress` is always the final count.
    pub fn finish(&self, mut result: Value, exit_code: i32) -> io::Result<()> {
        if let Some(result) = result.as_object_mut() {
            result.remove("files");
        }
        let mut state = self.state.lock().unwrap();
        if state.progress.pending() {
            state.progress.mark(Instant::now());
            let fields = state.progress.fields();
            Self::emit_locked(&mut state, "operation_progress", fields, false);
        }
        let fields = json!({
            "result": result,
            "exit_code": exit_code,
            "started_at": rfc3339_ms(state.started_at),
            "finished_at": rfc3339_ms(SystemTime::now()),
            "duration_ms": duration_ms(state.started),
        });
        Self::emit_locked(&mut state, "operation_finished", fields, true);
        state.finished = true;
        match state.error.as_ref() {
            Some(error) => Err(io::Error::new(error.kind(), error.to_string())),
            None => Ok(()),
        }
    }

    /// Byte callbacks report deltas; coalesced to at most ten progress events
    /// per second per transfer. The deltas also drive operation progress.
    pub fn progress(&self, path: &Path, total_bytes: u64) -> FileProgress {
        let path = path.to_path_buf();
        self.emit(
            "transfer_started",
            json!({"path": path, "total_bytes": total_bytes}),
            false,
        );
        let started = Instant::now();
        let progress = Arc::new(Mutex::new((0u64, Instant::now())));
        let bytes_progress = progress.clone();
        let bytes_stream = self.clone();
        let bytes_path = path.clone();
        let done_stream = self.clone();
        FileProgress {
            on_bytes: Box::new(move |delta| {
                bytes_stream.bytes_moved(delta);
                let mut progress = bytes_progress.lock().unwrap();
                progress.0 = progress.0.saturating_add(delta);
                if progress.1.elapsed() >= Duration::from_millis(100) {
                    bytes_stream.emit(
                        "transfer_progress",
                        json!({"path": bytes_path, "bytes": progress.0, "total_bytes": total_bytes}),
                        false,
                    );
                    progress.1 = Instant::now();
                }
            }),
            on_done: Box::new(move |success| {
                let progress = progress.lock().unwrap();
                done_stream.emit(
                    "transfer_finished",
                    json!({
                        "path": path,
                        "bytes": progress.0,
                        "total_bytes": total_bytes,
                        "success": success,
                        "duration_ms": duration_ms(started),
                    }),
                    false,
                );
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn events(capture: &Capture) -> Vec<Value> {
        String::from_utf8(capture.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn terminal_result_is_exact_and_late_events_are_ignored() {
        let capture = Capture::default();
        let stream = EventStream::new("get", Box::new(capture.clone()));
        let result =
            json!({"v": 1, "command": "get", "outcome": "partial", "counts": {"total": 0}});
        stream.finish(result.clone(), 1).unwrap();
        stream.waiting_for_auth("ignored", "ignored");
        stream.finish(json!({"ignored": true}), 0).unwrap();
        let events = events(&capture);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "operation_started");
        assert_eq!(events[1]["type"], "operation_finished");
        assert_eq!(events[1]["result"], result);
        assert_eq!(events[1]["exit_code"], 1);
    }

    #[test]
    fn file_rows_are_sent_once_and_left_out_of_the_finished_result() {
        let capture = Capture::default();
        let stream = EventStream::new("status", Box::new(capture.clone()));
        let first = json!({"path": "a.txt", "status": "current"});
        let second = json!({"path": "b.txt", "status": "absent"});
        stream.file_result(first.clone());
        stream.file_result(second.clone());
        let result = json!({"counts": {"total": 2}, "files": [first.clone(), second.clone()]});
        stream.record_streamed_result(result.clone());
        stream.finish(result, 0).unwrap();

        let events = events(&capture);
        let rows: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "file_result")
            .map(|event| event["result"].clone())
            .collect();
        assert_eq!(rows, vec![first, second]);
        let finished = events.last().unwrap();
        assert_eq!(finished["result"], json!({"counts": {"total": 2}}));
    }

    #[test]
    fn concurrent_callbacks_have_contiguous_sequence_numbers_and_complete_lines() {
        let capture = Capture::default();
        let stream = EventStream::new("add", Box::new(capture.clone()));
        std::thread::scope(|scope| {
            for index in 0..8 {
                let stream = stream.clone();
                scope.spawn(move || {
                    let callback = stream.progress(Path::new(&format!("file {index}\n.txt")), 15);
                    (callback.on_bytes)(7);
                    (callback.on_bytes)(8);
                    (callback.on_done)(true);
                });
            }
        });
        stream.finish(json!({"outcome": "complete"}), 0).unwrap();
        let events = events(&capture);
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["seq"], index);
            assert_eq!(event["v"], 1);
            assert_eq!(event["operation_id"], events[0]["operation_id"]);
        }
        let finishes: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "transfer_finished")
            .collect();
        assert_eq!(finishes.len(), 8);
        assert!(finishes.iter().all(|event| event["bytes"] == 15));
    }

    fn plan(files_total: u64, bytes_to_transfer: u64) -> OperationPlan {
        OperationPlan {
            files_total,
            bytes_total: bytes_to_transfer,
            files_to_transfer: files_total,
            bytes_to_transfer,
        }
    }

    #[test]
    fn every_event_has_a_timestamp_and_the_operation_elapsed_time() {
        let capture = Capture::default();
        let stream = EventStream::new("add", Box::new(capture.clone()));
        stream.planned(&plan(1, 4));
        stream.phase_started(Phase::Transfer);
        let transfer = stream.progress(Path::new("a.txt"), 4);
        (transfer.on_bytes)(4);
        (transfer.on_done)(true);
        stream.phase_finished(Phase::Transfer);
        stream.file_done();
        stream.finish(json!({}), 0).unwrap();
        let events = events(&capture);
        for event in &events {
            assert!(event["ts"].as_u64().unwrap() > 1_700_000_000_000, "{event}");
            assert!(event["elapsed_ms"].is_u64(), "{event}");
        }
        let finished = events.last().unwrap();
        assert!(finished["duration_ms"].is_u64());
        assert!(finished["started_at"].as_str().unwrap().ends_with('Z'));
        assert!(finished["finished_at"].as_str() >= finished["started_at"].as_str());
        let transfer_finished = events
            .iter()
            .find(|e| e["type"] == "transfer_finished")
            .unwrap();
        assert!(transfer_finished["duration_ms"].is_u64());
        assert_eq!(events[1]["type"], "operation_planned");
        assert_eq!(events[1]["bytes_to_transfer"], 4);
        assert_eq!(events[2]["phase"], "transfer");
    }

    fn progress_events(capture: &Capture) -> Vec<Value> {
        events(capture)
            .into_iter()
            .filter(|e| e["type"] == "operation_progress")
            .collect()
    }

    #[test]
    fn progress_is_throttled_and_the_final_count_is_always_reported() {
        let capture = Capture::default();
        let stream = EventStream::new("get", Box::new(capture.clone()));
        let started = Instant::now();
        stream.planned(&plan(5000, 0));
        for _ in 0..5000 {
            stream.file_done();
        }
        let throttled = progress_events(&capture).len() as u128;
        // At most one per 250 ms since the plan
        assert!(
            throttled <= started.elapsed().as_millis() / 250,
            "{throttled}"
        );
        stream.finish(json!({}), 0).unwrap();
        let progress = progress_events(&capture);
        let last = progress.last().unwrap();
        assert_eq!(last["files_done"], 5000);
        assert_eq!(last["files_total"], 5000);
        // The final count is the event right before the terminal one
        let all = events(&capture);
        assert_eq!(all[all.len() - 2], *last);
    }

    #[test]
    fn bytes_alone_report_progress_only_once_they_move_one_percent() {
        let capture = Capture::default();
        let stream = EventStream::new("add", Box::new(capture.clone()));
        stream.planned(&plan(1, 10_000));
        let transfer = stream.progress(Path::new("big.bin"), 10_000);
        std::thread::sleep(PROGRESS_INTERVAL + Duration::from_millis(50));
        (transfer.on_bytes)(50);
        assert!(progress_events(&capture).is_empty(), "0.5% is not enough");
        (transfer.on_bytes)(60);
        let progress = progress_events(&capture);
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0]["bytes_done"], 110);
        assert_eq!(progress[0]["bytes_total"], 10_000);
        // Right after one, nothing more for a while however much moves
        (transfer.on_bytes)(5_000);
        assert_eq!(progress_events(&capture).len(), 1);
        stream.finish(json!({}), 0).unwrap();
        let progress = progress_events(&capture);
        assert_eq!(progress.len(), 2);
        assert_eq!(progress[1]["bytes_done"], 5_110);
    }

    #[test]
    fn nothing_changed_since_the_last_progress_means_no_final_one() {
        let capture = Capture::default();
        let stream = EventStream::new("status", Box::new(capture.clone()));
        stream.planned(&plan(0, 0));
        stream.finish(json!({}), 0).unwrap();
        assert!(progress_events(&capture).is_empty());
    }

    #[test]
    fn broken_output_is_returned_without_panicking() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let stream = EventStream::new("status", Box::new(Broken));
        assert_eq!(
            stream.finish(json!({}), 0).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
