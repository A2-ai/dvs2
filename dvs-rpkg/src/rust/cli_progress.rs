//! Rust bindings for cli's C progress bar API.
//!
//! Resolves cli's C-callable functions via `R_GetCCallable` at first use.
//! Since the C API doesn't support changing total after creation, the bar
//! is recreated when the total grows (once per file start, not per chunk).

use std::sync::Once;

use miniextendr_api::ffi::{
    R_GetCCallable, R_NilValue, R_PreserveObject, R_ReleaseObject, SEXP,
};

type FnBar = unsafe extern "C" fn(*mut *mut i32, f64, SEXP) -> SEXP;
type FnAdd = unsafe extern "C" fn(SEXP, f64);
type FnSet = unsafe extern "C" fn(SEXP, f64);
type FnDone = unsafe extern "C" fn(SEXP);
type FnSetClear = unsafe extern "C" fn(SEXP, i32);

static INIT: Once = Once::new();
static mut FN_BAR: Option<FnBar> = None;
static mut FN_ADD: Option<FnAdd> = None;
static mut FN_SET: Option<FnSet> = None;
static mut FN_DONE: Option<FnDone> = None;
static mut FN_SET_CLEAR: Option<FnSetClear> = None;

fn ensure_init() {
    INIT.call_once(|| unsafe {
        miniextendr_api::RCall::from_cstr(c"loadNamespace")
            .arg(miniextendr_api::ffi::Rf_mkString(c"cli".as_ptr()))
            .eval_global()
            .expect("failed to load cli namespace");

        FN_BAR = Some(std::mem::transmute(R_GetCCallable(c"cli".as_ptr(), c"cli_progress_bar".as_ptr())));
        FN_ADD = Some(std::mem::transmute(R_GetCCallable(c"cli".as_ptr(), c"cli_progress_add".as_ptr())));
        FN_SET = Some(std::mem::transmute(R_GetCCallable(c"cli".as_ptr(), c"cli_progress_set".as_ptr())));
        FN_DONE = Some(std::mem::transmute(R_GetCCallable(c"cli".as_ptr(), c"cli_progress_done".as_ptr())));
        FN_SET_CLEAR = Some(std::mem::transmute(R_GetCCallable(c"cli".as_ptr(), c"cli_progress_set_clear".as_ptr())));
    });
}

fn create_bar(total: f64) -> SEXP {
    unsafe {
        let mut dummy: *mut i32 = std::ptr::null_mut();
        let bar = FN_BAR.unwrap_unchecked()(&mut dummy, total, R_NilValue);
        R_PreserveObject(bar);
        FN_SET_CLEAR.unwrap_unchecked()(bar, 0);
        bar
    }
}

fn release_bar(bar: SEXP) {
    unsafe {
        if bar != R_NilValue {
            FN_DONE.unwrap_unchecked()(bar);
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
        ensure_init();
        Self {
            bar: unsafe { R_NilValue },
            total: 0.0,
            current: 0.0,
        }
    }

    /// Grow total by `size` bytes. Recreates the bar with the new total.
    pub fn grow_total(&mut self, size: f64) {
        self.total += size;
        unsafe {
            if self.bar != R_NilValue {
                release_bar(self.bar);
            }
        }
        self.bar = create_bar(self.total);
        if self.current > 0.0 {
            unsafe { FN_SET.unwrap_unchecked()(self.bar, self.current) }
        }
    }

    /// Add `inc` bytes of progress.
    pub fn add(&mut self, inc: f64) {
        self.current += inc;
        unsafe {
            if self.bar != R_NilValue {
                FN_ADD.unwrap_unchecked()(self.bar, inc);
            }
        }
    }

    /// Terminate the progress bar.
    pub fn done(&mut self) {
        unsafe {
            if self.bar != R_NilValue {
                FN_DONE.unwrap_unchecked()(self.bar);
                R_ReleaseObject(self.bar);
                self.bar = R_NilValue;
            }
        }
    }
}

impl Drop for CliProgressBar {
    fn drop(&mut self) {
        unsafe {
            if self.bar != R_NilValue {
                R_ReleaseObject(self.bar);
            }
        }
    }
}
