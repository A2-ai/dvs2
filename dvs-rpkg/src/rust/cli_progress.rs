//! Rust bindings for cli's C progress bar API.
//!
//! Calls through thin C shims (cli_progress_shim.c) that wrap cli's
//! static inline functions. The shim is compiled by R's build system
//! and linked into the final shared object.

use miniextendr_api::{Preserved, SEXP};

#[cfg(not(test))]
unsafe extern "C" {
    fn cli_progress_bar_shim(total: f64, config: SEXP) -> SEXP;
    fn cli_progress_add_shim(bar: SEXP, inc: f64);
    fn cli_progress_set_shim(bar: SEXP, set: f64);
    fn cli_progress_done_shim(bar: SEXP);
    fn cli_progress_set_clear_shim(bar: SEXP, clear: i32);
}

// Stubs for `cargo test` — the real shims are C functions only available
// when linked into R's shared object.
#[cfg(test)]
unsafe fn cli_progress_bar_shim(_total: f64, _config: SEXP) -> SEXP {
    SEXP::nil()
}
#[cfg(test)]
unsafe fn cli_progress_add_shim(_bar: SEXP, _inc: f64) {}
#[cfg(test)]
unsafe fn cli_progress_set_shim(_bar: SEXP, _set: f64) {}
#[cfg(test)]
unsafe fn cli_progress_done_shim(_bar: SEXP) {}
#[cfg(test)]
unsafe fn cli_progress_set_clear_shim(_bar: SEXP, _clear: i32) {}

/// Progress bar that supports growing total via bar recreation.
pub struct CliProgressBar {
    /// The live cli bar, rooted while it exists.
    bar: Option<Preserved>,
    total: f64,
    current: f64,
}

impl CliProgressBar {
    pub fn new() -> Self {
        Self {
            bar: None,
            total: 0.0,
            current: 0.0,
        }
    }

    /// Grow total by `size` bytes. Recreates the bar with the new total.
    pub fn grow_total(&mut self, size: f64) {
        self.total += size;
        self.done();
        // cli allocates before it reads `config`, so keep it rooted until the
        // bar exists.
        let config = Preserved::new(miniextendr_api::list!("show_after" = 0.0));
        let bar = Preserved::new(unsafe { cli_progress_bar_shim(self.total, config.as_sexp()) });
        unsafe {
            cli_progress_set_clear_shim(bar.as_sexp(), 0);
            if self.current > 0.0 {
                cli_progress_set_shim(bar.as_sexp(), self.current);
            }
        }
        self.bar = Some(bar);
    }

    /// Add `inc` bytes of progress.
    pub fn add(&mut self, inc: f64) {
        self.current += inc;
        if let Some(bar) = &self.bar {
            unsafe { cli_progress_add_shim(bar.as_sexp(), inc) }
        }
    }

    /// Terminate the progress bar. Dropping the handle releases its root.
    pub fn done(&mut self) {
        if let Some(bar) = self.bar.take() {
            unsafe { cli_progress_done_shim(bar.as_sexp()) }
        }
    }
}

impl Drop for CliProgressBar {
    fn drop(&mut self) {
        self.done();
    }
}
