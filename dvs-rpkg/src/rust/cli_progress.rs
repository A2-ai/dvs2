//! Rust bindings for cli's C progress bar API.
//!
//! Calls through thin C shims (cli_progress_shim.c) that wrap cli's
//! static inline functions. The shim is compiled by R's build system
//! and linked into the final shared object.

use std::sync::Once;

use miniextendr_api::ffi::{R_PreserveObject, R_ReleaseObject, Rf_mkString, SEXP};

static ENSURE_CLI: Once = Once::new();

/// Ensure the cli namespace is loaded so R_GetCCallable can resolve the progress
/// functions (called lazily by the C shim's inline wrappers on first use).
fn ensure_cli_loaded() {
    ENSURE_CLI.call_once(|| unsafe {
        miniextendr_api::RCall::from_cstr(c"loadNamespace")
            .arg(Rf_mkString(c"cli".as_ptr()))
            .eval_base()
            .expect("failed to load cli namespace");
    });
}

unsafe extern "C" {
    fn cli_progress_bar_shim(total: f64, config: SEXP) -> SEXP;
    fn cli_progress_add_shim(bar: SEXP, inc: f64);
    fn cli_progress_set_shim(bar: SEXP, set: f64);
    fn cli_progress_done_shim(bar: SEXP);
    fn cli_progress_set_clear_shim(bar: SEXP, clear: i32);
}

/// Create a cli progress bar with the given total and show_after=0.
fn create_bar(total: f64) -> SEXP {
    unsafe {
        let config = miniextendr_api::list!("show_after" = 0.0);
        let bar = cli_progress_bar_shim(total, config.as_sexp());
        R_PreserveObject(bar);
        cli_progress_set_clear_shim(bar, 0);
        bar
    }
}

/// Finish and release a bar SEXP.
fn finish_bar(bar: SEXP) {
    unsafe {
        if bar != SEXP::nil() {
            cli_progress_done_shim(bar);
            R_ReleaseObject(bar);
        }
    }
}

/// Progress bar that supports growing total via bar recreation.
pub struct CliProgressBar {
    bar: SEXP,
    total: f64,
    current: f64,
}

impl CliProgressBar {
    pub fn new() -> Self {
        ensure_cli_loaded();
        Self {
            bar: SEXP::nil(),
            total: 0.0,
            current: 0.0,
        }
    }

    /// Grow total by `size` bytes. Recreates the bar with the new total.
    pub fn grow_total(&mut self, size: f64) {
        self.total += size;
        finish_bar(self.bar);
        self.bar = SEXP::nil(); // don't hold stale SEXP if create_bar panics
        self.bar = create_bar(self.total);
        if self.current > 0.0 {
            unsafe { cli_progress_set_shim(self.bar, self.current) }
        }
    }

    /// Add `inc` bytes of progress.
    pub fn add(&mut self, inc: f64) {
        self.current += inc;
        if self.bar != SEXP::nil() {
            unsafe { cli_progress_add_shim(self.bar, inc) }
        }
    }

    /// Terminate the progress bar.
    pub fn done(&mut self) {
        finish_bar(self.bar);
        self.bar = SEXP::nil();
    }
}

impl Drop for CliProgressBar {
    fn drop(&mut self) {
        if self.bar != SEXP::nil() {
            finish_bar(self.bar);
        }
    }
}
