//! Waiting for network work without blocking R: the work runs on its own thread while
//! R's thread checks for an interrupt (Esc, Ctrl-C) ten times a second. On an
//! interrupt R stops waiting and the work's result is dropped when it arrives; the
//! request itself is bounded by the HTTP agent's timeouts. R objects are only built
//! on R's thread, from the result.

use std::ffi::c_int;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::conditions::{ClientError, client_error};

const INTERRUPTED: &str = "interrupted";

unsafe extern "C" {
    // R_ext/GraphicsDevice.h: set by R's SIGINT handler (Esc, Ctrl-C) until R acts on
    // it. R_CheckUserInterrupt() would act on it by jumping out of the Rust frames
    // in between, and R_ToplevelExec() suspends interrupts, so read it instead.
    static mut R_interrupts_pending: c_int;
}

/// Whether the user asked R to stop; if so the request is taken, and the R side
/// signals the interrupt. Must run on R's thread.
fn interrupted() -> bool {
    // SAFETY: R's thread, between R calls; the signal handler only ever sets it to 1
    unsafe {
        let pending = std::ptr::addr_of_mut!(R_interrupts_pending);
        if pending.read_volatile() == 0 {
            return false;
        }
        pending.write_volatile(0);
        true
    }
}

/// Run `work` on another thread and wait for it, stopping with an `interrupted` error
/// when the user interrupts R.
pub(crate) fn off_r_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    loop {
        let received = rx.recv_timeout(Duration::from_millis(100));
        // An interrupt wins over a result that came in with it: the user asked to stop,
        // so the caller must not go on to commit what the result says
        if interrupted() {
            return Err(client_error(INTERRUPTED, "Interrupted"));
        }
        match received {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("the request stopped without an answer"));
            }
        }
    }
}

/// Whether `e` is the error [`off_r_thread`] stops with on an interrupt.
pub(crate) fn is_interrupted(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<ClientError>())
        .any(|c| c.code == INTERRUPTED)
}
