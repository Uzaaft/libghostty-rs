//! The logger is process-global and must not be changed while any other
//! thread uses libghostty. Tests run on parallel threads, so this lives in its
//! own test binary with a single test.
// These call into libghostty, which Miri cannot execute.
#![cfg(not(miri))]

use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

use libghostty_vt::{
    Terminal,
    log::{self, Level, Logger},
};

static FIRST: AtomicUsize = AtomicUsize::new(0);
static SECOND: AtomicUsize = AtomicUsize::new(0);

/// Counts the messages it receives in `count`.
struct Counting {
    count: &'static AtomicUsize,
    log_on_drop: bool,
}

impl Logger for Counting {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Counting {
    fn drop(&mut self) {
        if self.log_on_drop {
            // Destruction may use a terminal that makes libghostty log. The
            // replacement logger must already be callable without waiting for
            // this drop to finish.
            make_libghostty_log();
        }
    }
}

/// Feed a terminal a malformed set mode command, which libghostty logs as a
/// warning every time (so in release builds too).
fn make_libghostty_log() {
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal.vt_write(b"\x1b[>5h");
}

#[test]
fn replaced_logger_may_log_when_dropped() {
    // If replaced loggers are ever dropped under the logger lock again, the
    // log in `Counting::drop` deadlocks and this test would hang forever.
    // Fail it instead. The watchdog never calls into libghostty, so the
    // `set_logger` safety contract below still holds.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(30));
        // Not `eprintln!`: libtest captures that, and exiting skips printing
        // captured output, so the message would be lost.
        let _ = writeln!(
            std::io::stderr(),
            "logger_drop: timed out after 30s, likely deadlocked"
        );
        std::process::exit(1);
    });

    // SAFETY: No other thread in this test binary uses libghostty.
    unsafe {
        log::set_logger(Some(Box::new(Counting {
            count: &FIRST,
            log_on_drop: true,
        })))
    }
    .unwrap();
    make_libghostty_log();
    assert!(FIRST.load(Ordering::Relaxed) > 0, "nothing was logged");

    // Replacing the logger must allow its destructor to make libghostty log,
    // which then reaches the replacement.
    // SAFETY: As above.
    unsafe {
        log::set_logger(Some(Box::new(Counting {
            count: &SECOND,
            log_on_drop: false,
        })))
    }
    .unwrap();
    assert!(SECOND.load(Ordering::Relaxed) > 0, "drop log was lost");

    // SAFETY: As above.
    unsafe { log::set_logger(None) }.unwrap();
}
