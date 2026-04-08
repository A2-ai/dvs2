//! Rust bindings for cli's C progress bar API.
//!
//! cli exposes its progress bar functions via `R_GetCCallable`. The header
//! (`cli/progress.h`) contains `static inline` wrappers that resolve
//! function pointers at first call. We do the same from Rust.

use std::sync::Once;

use miniextendr_api::ffi::{R_GetCCallable, R_NilValue, R_PreserveObject, R_ReleaseObject, SEXP};

// Function pointer types matching cli's C API.
type CliProgressBarFn = unsafe extern "C" fn(*mut *mut i32, f64, SEXP) -> SEXP;
type CliProgressAddFn = unsafe extern "C" fn(SEXP, f64);
type CliProgressDoneFn = unsafe extern "C" fn(SEXP);
type CliProgressSetClearFn = unsafe extern "C" fn(SEXP, i32);

// Resolved function pointers (initialized once).
static INIT: Once = Once::new();
static mut FN_BAR: Option<CliProgressBarFn> = None;
static mut FN_ADD: Option<CliProgressAddFn> = None;
static mut FN_DONE: Option<CliProgressDoneFn> = None;
static mut FN_SET_CLEAR: Option<CliProgressSetClearFn> = None;

fn ensure_init() {
    INIT.call_once(|| unsafe {
        // Ensure cli's namespace is loaded so its C callables are registered.
        miniextendr_api::RCall::from_cstr(c"loadNamespace")
            .arg(miniextendr_api::ffi::Rf_mkString(c"cli".as_ptr()))
            .eval_global()
            .expect("failed to load cli namespace");

        FN_BAR = Some(std::mem::transmute(R_GetCCallable(
            c"cli".as_ptr(),
            c"cli_progress_bar".as_ptr(),
        )));
        FN_ADD = Some(std::mem::transmute(R_GetCCallable(
            c"cli".as_ptr(),
            c"cli_progress_add".as_ptr(),
        )));
        FN_DONE = Some(std::mem::transmute(R_GetCCallable(
            c"cli".as_ptr(),
            c"cli_progress_done".as_ptr(),
        )));
        FN_SET_CLEAR = Some(std::mem::transmute(R_GetCCallable(
            c"cli".as_ptr(),
            c"cli_progress_set_clear".as_ptr(),
        )));
    });
}

/// RAII wrapper around a cli progress bar.
pub struct CliProgressBar {
    bar: SEXP,
}

impl CliProgressBar {
    /// Create a new progress bar. Pass `f64::NAN` for unknown total.
    pub fn new(total: f64) -> Self {
        ensure_init();
        unsafe {
            let create = FN_BAR.unwrap_unchecked();
            // cli_progress_bar takes a *mut *mut i32 for its should_tick timer.
            // We don't use CLI_SHOULD_TICK from Rust — pass a dummy.
            let mut dummy: *mut i32 = std::ptr::null_mut();
            let bar = create(&mut dummy, total, R_NilValue);
            R_PreserveObject(bar);

            let set_clear = FN_SET_CLEAR.unwrap_unchecked();
            set_clear(bar, 0);

            Self { bar }
        }
    }

    /// Add `inc` bytes of progress. cli handles render throttling internally.
    pub fn add(&self, inc: f64) {
        unsafe {
            let f = FN_ADD.unwrap_unchecked();
            f(self.bar, inc);
        }
    }

    /// Terminate the progress bar.
    pub fn done(&self) {
        unsafe {
            let f = FN_DONE.unwrap_unchecked();
            f(self.bar);
        }
    }
}

impl Drop for CliProgressBar {
    fn drop(&mut self) {
        unsafe {
            if self.bar != R_NilValue {
                R_ReleaseObject(self.bar);
                self.bar = R_NilValue;
            }
        }
    }
}
