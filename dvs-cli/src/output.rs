//! Versioned machine output. Messages are for people; codes are the API.
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

use anyhow::Error;
use dvs::operation::{duration_ms, rfc3339_ms};
use dvs::{ByteTotals, Counts, ErrorCode, GetVerdict, RunOutcome};
use serde_json::{Value, json};

/// Exit codes, shared by every command; only `get` uses 3.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFLICTS: i32 = 3;
/// A cancelled run, conventional for SIGINT (128 + 2).
pub const EXIT_CANCELLED: i32 = 130;

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
    } else if error.downcast_ref::<dvs::config::ConfigError>().is_some() {
        ErrorCode::InvalidConfiguration
    } else if let Some(code) = dvs::backend_error_code(error) {
        code
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        ErrorCode::IoError
    } else {
        ErrorCode::OperationFailed
    }
}

/// The exit status of a run that reported rows: get passes its verdict, add
/// and status their outcome.
pub fn run_exit_code(verdict: impl Into<GetVerdict>) -> i32 {
    match verdict.into() {
        GetVerdict::Done => EXIT_OK,
        GetVerdict::Failed => EXIT_FAILED,
        GetVerdict::HeldBack => EXIT_CONFLICTS,
        GetVerdict::Cancelled => EXIT_CANCELLED,
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

pub fn rows_envelope(
    command: &str,
    dry_run: bool,
    files: Vec<Value>,
    counts: Counts,
    outcome: RunOutcome,
) -> Value {
    let mut value = envelope(command, dry_run);
    value["outcome"] = json!(outcome);
    value["counts"] = json!(counts);
    value["files"] = json!(files);
    value
}

pub fn problem(command: &str, dry_run: bool, error: &Error) -> Value {
    let mut value = match command {
        "add" | "get" | "status" | "availability" | "history" => rows_envelope(
            command,
            dry_run,
            vec![],
            Counts::default(),
            RunOutcome::Failed,
        ),
        _ => envelope(command, dry_run),
    };
    if matches!(command, "add" | "get") {
        value["bytes"] = json!(ByteTotals::default());
    }
    value["outcome"] = json!("failed");
    value["error"] = error_json(error_code(error), format!("{error:#}"));
    value
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

/// Streamed rows go through here too, so they match the result's rows.
/// `has_code`: the row failed or was cancelled.
pub fn file_result(mut value: Value, has_code: bool) -> Value {
    if !has_code {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_exits_130_whatever_the_command() {
        assert_eq!(run_exit_code(RunOutcome::Cancelled), EXIT_CANCELLED);
        assert_eq!(run_exit_code(GetVerdict::Cancelled), EXIT_CANCELLED);
        assert_eq!(run_exit_code(RunOutcome::Partial), EXIT_FAILED);
        assert_eq!(run_exit_code(RunOutcome::Complete), EXIT_OK);
        assert_eq!(run_exit_code(GetVerdict::HeldBack), EXIT_CONFLICTS);
    }
}
