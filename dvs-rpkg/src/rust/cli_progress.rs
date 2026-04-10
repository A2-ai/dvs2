//! Rust bindings for cli's C progress bar API.
//!
//! Resolves cli's C-callable functions via `R_GetCCallable` at first use.
//! Since the C API doesn't support changing total after creation, the bar
//! is recreated when the total grows (once per file start, not per chunk).

use std::sync::OnceLock;

use miniextendr_api::ffi::{
    R_GetCCallable, R_NilValue, R_PreserveObject, R_ReleaseObject, Rf_ScalarReal, Rf_protect,
    Rf_unprotect, SEXP, SEXPTYPE,
};

type FnBar = unsafe extern "C" fn(*mut *mut i32, f64, SEXP) -> SEXP;
type FnAdd = unsafe extern "C" fn(SEXP, f64);
type FnSet = unsafe extern "C" fn(SEXP, f64);
type FnDone = unsafe extern "C" fn(SEXP);
type FnSetClear = unsafe extern "C" fn(SEXP, i32);

struct CliFns {
    bar: FnBar,
    add: FnAdd,
    set: FnSet,
    done: FnDone,
    set_clear: FnSetClear,
}

static CLI_FNS: OnceLock<CliFns> = OnceLock::new();

#[allow(clippy::missing_transmute_annotations)]
fn fns() -> &'static CliFns {
    CLI_FNS.get_or_init(|| unsafe {
        miniextendr_api::RCall::from_cstr(c"loadNamespace")
            .arg(miniextendr_api::ffi::Rf_mkString(c"cli".as_ptr()))
            .eval_global()
            .expect("failed to load cli namespace");

        CliFns {
            bar: std::mem::transmute(R_GetCCallable(
                c"cli".as_ptr(),
                c"cli_progress_bar".as_ptr(),
            )),
            add: std::mem::transmute(R_GetCCallable(
                c"cli".as_ptr(),
                c"cli_progress_add".as_ptr(),
            )),
            set: std::mem::transmute(R_GetCCallable(
                c"cli".as_ptr(),
                c"cli_progress_set".as_ptr(),
            )),
            done: std::mem::transmute(R_GetCCallable(
                c"cli".as_ptr(),
                c"cli_progress_done".as_ptr(),
            )),
            set_clear: std::mem::transmute(R_GetCCallable(
                c"cli".as_ptr(),
                c"cli_progress_set_clear".as_ptr(),
            )),
        }
    })
}

/// Create a cli progress bar with the given total and show_after=0.
fn create_bar(total: f64) -> SEXP {
    unsafe {
        let f = fns();
        // Build config: list(show_after = 0)
        let val = Rf_protect(Rf_ScalarReal(0.0));
        let names = Rf_protect(miniextendr_api::ffi::Rf_mkString(c"show_after".as_ptr()));
        let config = Rf_protect(miniextendr_api::ffi::Rf_allocVector(SEXPTYPE::VECSXP, 1));
        miniextendr_api::ffi::SET_VECTOR_ELT(config, 0, val);
        miniextendr_api::ffi::Rf_setAttrib(config, miniextendr_api::ffi::R_NamesSymbol, names);

        let mut dummy: *mut i32 = std::ptr::null_mut();
        let bar = (f.bar)(&mut dummy, total, config);
        R_PreserveObject(bar);
        (f.set_clear)(bar, 0);

        Rf_unprotect(3); // val, names, config — bar is preserved
        bar
    }
}

/// Finish and release a bar SEXP.
fn finish_bar(bar: SEXP) {
    unsafe {
        if bar != R_NilValue {
            (fns().done)(bar);
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
        // Eagerly resolve cli C functions so create_bar doesn't fail later.
        let _ = fns();
        Self {
            bar: unsafe { R_NilValue },
            total: 0.0,
            current: 0.0,
        }
    }

    /// Grow total by `size` bytes. Recreates the bar with the new total.
    pub fn grow_total(&mut self, size: f64) {
        self.total += size;
        finish_bar(self.bar);
        self.bar = unsafe { R_NilValue }; // don't hold stale SEXP if create_bar panics
        self.bar = create_bar(self.total);
        if self.current > 0.0 {
            unsafe { (fns().set)(self.bar, self.current) }
        }
    }

    /// Add `inc` bytes of progress.
    pub fn add(&mut self, inc: f64) {
        self.current += inc;
        unsafe {
            if self.bar != R_NilValue {
                (fns().add)(self.bar, inc);
            }
        }
    }

    /// Terminate the progress bar.
    pub fn done(&mut self) {
        finish_bar(self.bar);
        self.bar = unsafe { R_NilValue };
    }
}

impl Drop for CliProgressBar {
    fn drop(&mut self) {
        if self.bar != unsafe { R_NilValue } {
            finish_bar(self.bar);
        }
    }
}
