//! Cancellation: turn SIGINT/SIGTERM/SIGHUP, and with `--emit-events`
//! the end of stdin (Unix only), into the operation's cancel flag.
//!
//! The library only stops *starting* files once the flag is set; files in
//! flight finish atomically, the full result is still written, and the CLI
//! exits 130. A second signal is the escape hatch for a file that takes too
//! long to finish: it exits 130 at once, without a result.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use dvs::CancelToken;

use crate::output::EXIT_CANCELLED;

/// Install the handlers for one cancellable run. Only `add`, `get` and
/// `status` call this: every other command keeps the default "a signal kills
/// the process" behaviour, so Ctrl-C still stops `dvs login` polling.
pub fn install(token: &CancelToken, watch_stdin: bool) {
    // A signal the process was started with ignored (`nohup`, a shell's
    // background job without job control) must stay ignored: installing a
    // handler would make it cancel where it used to do nothing at all.
    #[cfg(unix)]
    let ignored = unix::ignored_signals();
    let signalled = AtomicBool::new(false);
    let on_signal = token.clone();
    // `termination` also covers SIGTERM (and SIGHUP) on Unix and
    // Ctrl-Break/close on Windows. A failure to install leaves the default
    // behaviour, which is no worse than before cancellation existed.
    if let Err(error) = ctrlc::set_handler(move || {
        if signalled.swap(true, Ordering::SeqCst) {
            std::process::exit(EXIT_CANCELLED);
        }
        on_signal.cancel();
    }) {
        log::warn!("Could not install the cancellation handler: {error}");
    }
    #[cfg(unix)]
    unix::ignore(&ignored);
    if watch_stdin && stdin_is_a_channel() {
        let on_eof = token.clone();
        std::thread::spawn(move || {
            // Whatever the client writes is ignored; only the end matters. A
            // read error means the other end is gone too.
            let mut stdin = std::io::stdin().lock();
            let mut buffer = [0u8; 4096];
            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            on_eof.cancel();
        });
    }
}

/// Whether stdin is a pipe or socket a client holds open, so its end means
/// "the client went away". `/dev/null` (the default for most process APIs),
/// a regular file and a terminal are at EOF, or never reach it, for reasons
/// that have nothing to do with cancelling, so they are not watched.
#[cfg(unix)]
fn stdin_is_a_channel() -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::FileTypeExt;
    let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
        return false;
    };
    std::fs::File::from(fd)
        .metadata()
        .is_ok_and(|m| m.file_type().is_fifo() || m.file_type().is_socket())
}

/// Elsewhere (Windows) there is no portable way to tell a pipe a client
/// holds open from the null device or a file, so stdin is never watched:
/// end-of-stdin cancel is Unix only, and Windows clients cancel with
/// Ctrl-C/Ctrl-Break.
#[cfg(not(unix))]
fn stdin_is_a_channel() -> bool {
    false
}

/// Signal dispositions, for keeping inherited `SIG_IGN`s ignored.
#[cfg(unix)]
mod unix {
    /// The signals the cancellation handler covers on Unix.
    const HANDLED: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    /// The handled signals that are ignored right now.
    pub fn ignored_signals() -> Vec<libc::c_int> {
        HANDLED
            .into_iter()
            .filter(|&signal| {
                // SAFETY: a null new action only reads the current one into
                // `current`, which is plain data the call fully initializes.
                let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
                let read = unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) };
                read == 0 && current.sa_sigaction == libc::SIG_IGN
            })
            .collect()
    }

    /// Ignore `signals` again, over the handler just installed.
    pub fn ignore(signals: &[libc::c_int]) {
        for &signal in signals {
            // SAFETY: setting SIG_IGN installs no code to run in the handler.
            if unsafe { libc::signal(signal, libc::SIG_IGN) } == libc::SIG_ERR {
                log::warn!("Could not keep signal {signal} ignored");
            }
        }
    }
}
