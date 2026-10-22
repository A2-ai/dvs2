//! Serialized, flushed NDJSON output for subprocess clients.
//!
//! Transfer callbacks describe transfer work only. The terminal command result
//! remains authoritative: a transfer can finish before installation fails.
//!
//! Every event carries `ts` (unix milliseconds) and `elapsed_ms` (since the
//! operation started).

use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dvs::FileProgress;
use dvs::operation::{duration_ms, rfc3339_ms, unix_ms};
use serde_json::{Value, json};

static ACTIVE: OnceLock<EventStream> = OnceLock::new();

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
            })),
        };
        stream.emit("operation_started", json!({"command": command}), false);
        stream
    }

    fn emit(&self, kind: &str, fields: Value, terminal: bool) {
        let mut state = self.state.lock().unwrap();
        Self::emit_locked(&mut state, kind, fields, terminal);
    }

    /// Write one event with the state already locked, so a caller can read
    /// shared state and emit from it atomically.
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

    /// Write exactly one terminal event. This is the same complete document
    /// emitted by `--json`, including startup errors and per-file failures.
    pub fn finish(&self, result: Value, exit_code: i32) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
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

    /// Byte callbacks report deltas. Coalesce them to at most ten progress
    /// events per second per transfer, always flushing the final count.
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
    fn every_event_has_a_timestamp_and_the_operation_elapsed_time() {
        let capture = Capture::default();
        let stream = EventStream::new("add", Box::new(capture.clone()));
        let transfer = stream.progress(Path::new("a.txt"), 4);
        (transfer.on_bytes)(4);
        (transfer.on_done)(true);
        stream.file_result(json!({"path": "a.txt"}));
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
    }

    #[test]
    fn terminal_result_is_exact_and_late_events_are_ignored() {
        let capture = Capture::default();
        let stream = EventStream::new("get", Box::new(capture.clone()));
        let result = json!({"v": 1, "command": "get", "outcome": "partial", "files": []});
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
    fn streamed_file_results_are_not_repeated_at_finish() {
        let capture = Capture::default();
        let stream = EventStream::new("status", Box::new(capture.clone()));
        let first = json!({"path": "a.txt", "status": "current"});
        let second = json!({"path": "b.txt", "status": "absent"});
        stream.file_result(first.clone());
        stream.file_result(second.clone());
        let result = json!({"files": [first.clone(), second.clone()]});
        stream.record_streamed_result(result.clone());
        stream.finish(result, 0).unwrap();

        let events = events(&capture);
        let rows: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "file_result")
            .map(|event| event["result"].clone())
            .collect();
        assert_eq!(rows, vec![first, second]);
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
