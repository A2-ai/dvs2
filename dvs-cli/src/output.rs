//! Versioned machine output. Messages are for people; codes are the API.
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

use anyhow::Error;
use dvs::operation::{duration_ms, rfc3339_ms};
use serde_json::{Value, json};

/// When this run started, for the timing every result carries (C2). One run
/// is one operation, so this is set once, before anything is emitted.
#[derive(Clone, Copy)]
pub struct Clock {
    pub started: Instant,
    pub started_at: SystemTime,
}

static CLOCK: OnceLock<Clock> = OnceLock::new();

pub fn start_clock(started: Instant, started_at: SystemTime) {
    let _ = CLOCK.set(Clock {
        started,
        started_at,
    });
}

/// The run's clock; "now" if the run failed before it could start one
/// (argument errors).
pub fn clock() -> Clock {
    *CLOCK.get_or_init(|| Clock {
        started: Instant::now(),
        started_at: SystemTime::now(),
    })
}

/// Add `started_at`, `finished_at` (RFC 3339 UTC, milliseconds) and
/// `duration_ms` to a result document, finishing it now.
pub fn stamp(value: &mut Value) {
    let clock = clock();
    value["started_at"] = json!(rfc3339_ms(clock.started_at));
    value["finished_at"] = json!(rfc3339_ms(SystemTime::now()));
    value["duration_ms"] = json!(duration_ms(clock.started));
}

#[derive(Debug)]
pub struct CodedError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

pub fn coded(code: &'static str, message: impl Into<String>) -> Error {
    CodedError {
        code,
        message: message.into(),
    }
    .into()
}

pub fn error_code(error: &Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<CodedError>() {
        error.code
    } else if error
        .downcast_ref::<dvs::globbing::NoFilesSelected>()
        .is_some()
    {
        "no_files_selected"
    } else if error.downcast_ref::<dvs::config::ConfigError>().is_some() {
        "invalid_configuration"
    } else if let Some(code) = dvs::backend_error_code(error) {
        code.as_str()
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        "io_error"
    } else {
        "operation_failed"
    }
}

pub fn exit_code(error: &Error) -> i32 {
    match error_code(error) {
        "invalid_arguments" | "invalid_configuration" => crate::get_cmd::EXIT_USAGE,
        _ => 1,
    }
}

pub fn envelope(command: &str, dry_run: bool, files: Vec<Value>, failed: usize) -> Value {
    let total = files.len();
    json!({
        "v": 1,
        "command": command,
        "outcome": if failed == 0 {
            "complete"
        } else if failed == total {
            "failed"
        } else {
            "partial"
        },
        "dry_run": dry_run,
        "counts": {
            "total": total,
            "succeeded": total - failed,
            "failed": failed,
        },
        "problem": null,
        "error": null,
        "files": files,
    })
}

pub fn problem(command: &str, dry_run: bool, error: &Error) -> Value {
    let mut value = if command == "get" {
        crate::get_cmd::problem_json(&format!("{error:#}"), dry_run)
    } else {
        envelope(command, dry_run, vec![], 0)
    };
    if command == "add" {
        // C8: add results always carry byte totals, nothing moved here
        value["bytes"] = json!(dvs::ByteTotals::default());
    }
    value["outcome"] = json!("failed");
    value["problem"] = json!(format!("{error:#}"));
    value["error"] = json!({
        "code": error_code(error),
        "message": format!("{error:#}"),
    });
    value
}

/// Single rendering seam for JSON and, in future, terminal event results.
/// Every result leaves through here, which adds its timing.
pub fn emit(mut value: Value) {
    stamp(&mut value);
    println!("{value}");
}

pub fn file_results(rows: impl IntoIterator<Item = (Value, bool)>) -> (Vec<Value>, usize) {
    let mut failed = 0;
    let files = rows
        .into_iter()
        .map(|(mut value, is_error)| {
            failed += usize::from(is_error);
            value["code"] = if is_error {
                json!("file_failed")
            } else {
                Value::Null
            };
            value
        })
        .collect();
    (files, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamped_results_have_rfc3339_times_and_a_duration() {
        let mut result = envelope("info", false, vec![], 0);
        stamp(&mut result);
        let started = result["started_at"].as_str().unwrap();
        let finished = result["finished_at"].as_str().unwrap();
        assert!(started.ends_with('Z') && started.len() == 24, "{started}");
        assert!(finished >= started);
        assert!(result["duration_ms"].is_u64());
    }
}
