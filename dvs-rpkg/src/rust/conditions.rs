//! Errors as R conditions. Every error raised from Rust carries a stable `code`: the
//! server's, one this package gives its own errors, or `error`. The R wrappers turn
//! it into a `dvs_error_<code>` condition (see `R/dvs-conditions.R`).

use std::fmt;

use dvs::remote::{ApiError, api_error};

/// R's `NA_integer_`: the status of an error no server answered.
const NA_INTEGER: i32 = i32::MIN;

/// An error this package raises itself, with a stable code like the server's.
#[derive(Debug)]
pub(crate) struct ClientError {
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ClientError {}

pub(crate) fn client_error(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    ClientError {
        code,
        message: message.into(),
    }
    .into()
}

/// The dvs crate's hints name CLI commands; in R, name the function that does the same.
fn for_r(message: &str) -> String {
    message.replace("`dvs login`", "`dvs_login()`")
}

/// Raise `e` in R as a `dvs_rust_error` condition with the fields `code`, `status`
/// (NA when no server answered) and `request_id` (empty when there is none).
pub(crate) fn raise(e: anyhow::Error) -> ! {
    let client = e.chain().find_map(|c| c.downcast_ref::<ClientError>());
    let (code, status, request_id) = match (client, api_error(&e)) {
        (Some(c), _) => (c.code.to_string(), NA_INTEGER, String::new()),
        (None, Some(a)) => (
            a.code.clone(),
            i32::from(a.status),
            a.request_id.clone().unwrap_or_default(),
        ),
        (None, None) => ("error".to_string(), NA_INTEGER, String::new()),
    };
    // The server's own words, without the " (code)" its Display adds: the code is a
    // field of the condition
    let message = match e.downcast_ref::<ApiError>() {
        Some(a) => a.message.clone(),
        None => format!("{e:#}"),
    };
    miniextendr_api::error!(
        class = "dvs_rust_error",
        data = [
            ("code", code),
            ("status", status),
            ("request_id", request_id)
        ],
        "{}",
        for_r(&message)
    )
}

/// The value, or the error raised in R.
pub(crate) fn or_raise<T>(result: anyhow::Result<T>) -> T {
    result.unwrap_or_else(|e| raise(e))
}
