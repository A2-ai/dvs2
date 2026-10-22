//! Versioned machine output. Messages are for people; codes are the API.
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

use anyhow::Error;
use dvs::operation::{duration_ms, rfc3339_ms};
use dvs::{ByteTotals, Counts, ErrorCode};
use serde_json::{Value, json};

/// Exit codes, shared by every command; only `get` uses 3.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFLICTS: i32 = 3;

/// When this run started, for the timing every result carries.
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
    pub code: ErrorCode,
    pub message: String,
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

pub fn coded(code: ErrorCode, message: impl Into<String>) -> Error {
    CodedError {
        code,
        message: message.into(),
    }
    .into()
}

pub fn error_code(error: &Error) -> ErrorCode {
    if let Some(error) = error.downcast_ref::<CodedError>() {
        error.code
    } else if error
        .downcast_ref::<dvs::globbing::NoFilesSelected>()
        .is_some()
    {
        ErrorCode::NoFilesSelected
    } else if error.downcast_ref::<dvs::config::ConfigError>().is_some() {
        ErrorCode::InvalidConfiguration
    } else if error
        .downcast_ref::<crate::read_cmd::QueryInputError>()
        .is_some()
    {
        ErrorCode::InvalidArguments
    } else if let Some(code) = dvs::backend_error_code(error) {
        code
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        ErrorCode::IoError
    } else {
        ErrorCode::OperationFailed
    }
}

/// The exit status of a run that failed before it could report rows.
pub fn exit_code(error: &Error) -> i32 {
    match error_code(error) {
        ErrorCode::InvalidArguments | ErrorCode::InvalidConfiguration => EXIT_USAGE,
        _ => EXIT_FAILED,
    }
}

pub fn error_json(code: ErrorCode, message: impl Into<String>) -> Value {
    json!({"code": code, "message": message.into(), "hint": null})
}

/// The envelope of a command without rows.
pub fn envelope(command: &str, dry_run: bool) -> Value {
    json!({
        "v": 1,
        "command": command,
        "outcome": "complete",
        "dry_run": dry_run,
        "error": null,
    })
}

pub fn rows_envelope(command: &str, dry_run: bool, files: Vec<Value>, counts: Counts) -> Value {
    let mut value = envelope(command, dry_run);
    value["outcome"] = json!(if counts.failed == 0 {
        "complete"
    } else if counts.failed == counts.total {
        "failed"
    } else {
        "partial"
    });
    value["counts"] = json!(counts);
    value["files"] = json!(files);
    value
}

pub fn problem(command: &str, dry_run: bool, error: &Error) -> Value {
    let mut value = match command {
        "add" | "get" | "status" | "hash" | "availability" | "history" => {
            rows_envelope(command, dry_run, vec![], Counts::default())
        }
        _ => envelope(command, dry_run),
    };
    if matches!(command, "add" | "get") {
        value["bytes"] = json!(ByteTotals::default());
    }
    value["outcome"] = json!("failed");
    value["error"] = error_json(error_code(error), format!("{error:#}"));
    value
}

/// Single rendering seam for JSON and, in future, terminal event results.
/// Adds the timing every result carries.
pub fn emit(mut value: Value) {
    stamp(&mut value);
    println!("{value}");
}

pub fn file_results(rows: impl IntoIterator<Item = (Value, bool)>) -> (Vec<Value>, usize) {
    let mut failed = 0;
    let files = rows
        .into_iter()
        .map(|(value, is_error)| {
            failed += usize::from(is_error);
            file_result(value, is_error)
        })
        .collect();
    (files, failed)
}

pub fn file_result(mut value: Value, is_error: bool) -> Value {
    if !is_error {
        value["code"] = Value::Null;
    } else if value.get("code").is_none_or(Value::is_null) {
        value["code"] = json!(ErrorCode::FileFailed);
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
