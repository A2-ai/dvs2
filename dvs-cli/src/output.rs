//! Versioned machine output. Messages are for people; codes are the API.
use anyhow::Error;
use serde_json::{Value, json};

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
    } else if error
        .downcast_ref::<dvs::backends::server::Denied>()
        .is_some()
    {
        "access_denied"
    } else if error
        .downcast_ref::<dvs::backends::server::Unreachable>()
        .is_some()
    {
        "server_unavailable"
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
    value["outcome"] = json!("failed");
    value["problem"] = json!(format!("{error:#}"));
    value["error"] = json!({
        "code": error_code(error),
        "message": format!("{error:#}"),
    });
    value
}

/// Single rendering seam for JSON and, in future, terminal event results.
pub fn emit(value: Value) {
    if let Some(events) = crate::events::active() {
        events.record_result(value);
    } else {
        println!("{value}");
    }
}

/// Record a result whose file rows have already been streamed as they completed.
pub fn emit_streamed(value: Value) {
    crate::events::active()
        .expect("streamed file rows require event mode")
        .record_streamed_result(value);
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
