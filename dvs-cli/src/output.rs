//! Versioned machine output. Messages are for people; codes are the API.
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

use anyhow::Error;
use dvs::operation::{duration_ms, rfc3339_ms};
use serde_json::{Value, json};

/// When this run's operation started, for the timing every result carries.
/// Set once, from the operation context, before anything is emitted.
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

/// The run's clock; "now" for argument errors, which fail before it starts.
pub fn clock() -> Clock {
    *CLOCK.get_or_init(|| Clock {
        started: Instant::now(),
        started_at: SystemTime::now(),
    })
}

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
    } else if error
        .downcast_ref::<crate::read_cmd::QueryInputError>()
        .is_some()
    {
        "invalid_arguments"
    } else if let Some(code) = dvs::backend_error_code(error) {
        code.as_str()
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        "io_error"
    } else {
        "operation_failed"
    }
}

/// The exit status of a run that failed before it could report rows.
/// Cancelled (130) wins over every other status.
pub fn exit_code(error: &Error) -> i32 {
    match error_code(error) {
        "invalid_arguments" | "invalid_configuration" => crate::get_cmd::EXIT_USAGE,
        _ => crate::get_cmd::EXIT_FAILED,
    }
}

pub fn error_json(code: &str, message: impl Into<String>) -> Value {
    json!({"code": code, "message": message.into(), "hint": null})
}

/// `failed` counts failed rows; `cancelled` rows count in `total` only. A
/// cancelled run's `outcome` is set by the caller ([`cancelled`]).
pub fn envelope(command: &str, dry_run: bool, files: Vec<Value>, failed: usize) -> Value {
    let total = files.len();
    let cancelled = files.iter().filter(|row| is_cancelled_row(row)).count();
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
            "succeeded": total - failed - cancelled,
            "failed": failed,
        },
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
        value["bytes"] = json!(dvs::ByteTotals::default());
    }
    value["outcome"] = json!("failed");
    value["error"] = error_json(error_code(error), format!("{error:#}"));
    value
}

pub fn is_cancelled_row(row: &Value) -> bool {
    row["code"] == "cancelled"
}

/// The cancel flag was set before the operation finished, whatever its rows say.
pub fn cancelled(value: &mut Value) {
    value["outcome"] = json!("cancelled");
}

/// Single rendering seam for JSON and terminal event results. Adds the
/// timing every result carries.
pub fn emit(mut value: Value) {
    stamp(&mut value);
    if let Some(events) = crate::events::active() {
        events.record_result(value);
    } else {
        println!("{value}");
    }
}

/// Record a result whose file rows have already been streamed as they
/// completed; without events, the same as [`emit`].
pub fn emit_streamed(mut value: Value) {
    match crate::events::active() {
        Some(events) => {
            stamp(&mut value);
            events.record_streamed_result(value);
        }
        None => emit(value),
    }
}

/// Cancelled rows are errors to the library but not failures here.
pub fn file_results(rows: impl IntoIterator<Item = (Value, bool)>) -> (Vec<Value>, usize) {
    let mut failed = 0;
    let files = rows
        .into_iter()
        .map(|(value, is_error)| {
            let value = file_result(value, is_error);
            failed += usize::from(is_error && !is_cancelled_row(&value));
            value
        })
        .collect();
    (files, failed)
}

/// Streamed rows go through here too, so they match the result's rows.
pub fn file_result(mut value: Value, is_error: bool) -> Value {
    if !is_error {
        value["code"] = Value::Null;
    } else if value.get("code").is_none_or(Value::is_null) {
        value["code"] = json!("file_failed");
    }
    row_error_fields(value)
}

/// A row's keys never depend on whether it failed.
pub fn row_error_fields(mut value: Value) -> Value {
    for key in ["code", "message", "hint"] {
        if value.get(key).is_none() {
            value[key] = Value::Null;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_rows_count_in_total_only() {
        let (files, failed) = file_results([
            (json!({"path": "ok"}), false),
            (json!({"path": "bad", "code": "io_error"}), true),
            (json!({"path": "later", "code": "cancelled"}), true),
        ]);
        assert_eq!(failed, 1);
        let mut result = envelope("add", false, files, failed);
        assert_eq!(
            result["counts"],
            json!({"total": 3, "succeeded": 1, "failed": 1})
        );
        assert_eq!(result["files"][0]["code"], Value::Null);
        assert_eq!(result["files"][2]["code"], "cancelled");
        cancelled(&mut result);
        assert_eq!(result["outcome"], "cancelled");
    }
}
