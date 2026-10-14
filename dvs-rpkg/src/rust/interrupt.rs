//! Waiting for network work without blocking R: the work runs on its own thread while
//! R's thread checks for an interrupt (Esc, Ctrl-C) ten times a second. On an
//! interrupt R stops waiting and the work's result is dropped when it arrives; the
//! request itself is bounded by the HTTP agent's timeouts. R objects are only built
//! on R's thread, from the result.

use std::ffi::c_int;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use anyhow::{Result, anyhow};
use dvs::auth;

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
    let abandoned = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&abandoned);
    std::thread::spawn(move || {
        // Work R stopped waiting for changes no credentials from then on
        let result = auth::with_check(
            move || {
                if stop.load(Ordering::SeqCst) {
                    Err(client_error(INTERRUPTED, "Interrupted"))
                } else {
                    Ok(())
                }
            },
            work,
        );
        let _ = tx.send(result);
    });
    loop {
        let received = rx.recv_timeout(Duration::from_millis(100));
        // An interrupt wins over a result that came in with it: the user asked to stop,
        // so the caller must not go on to commit what the result says
        if interrupted() {
            abandoned.store(true, Ordering::SeqCst);
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

/// Run `work`, which changes the credentials store, on R's thread: waiting for the
/// store's locks stops on an interrupt, and so does a change not yet written when the
/// locks are held.
pub(crate) fn on_r_thread_credentials<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    auth::with_check(
        || {
            if interrupted() {
                Err(client_error(INTERRUPTED, "Interrupted"))
            } else {
                Ok(())
            }
        },
        work,
    )
}

/// Whether `e` is the error [`off_r_thread`] stops with on an interrupt.
pub(crate) fn is_interrupted(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<ClientError>())
        .any(|c| c.code == INTERRUPTED)
}
