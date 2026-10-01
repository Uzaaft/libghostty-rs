//! The secure random source is process-global and must not be changed while
//! any other thread uses libghostty. Tests run on parallel threads, so this
//! lives in its own test binary with a single test.
// These call into libghostty, which Miri cannot execute.
#![cfg(not(miri))]

use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use libghostty_vt::{
    Error, Terminal,
    paste::Options,
    sys::set_secure_random_source,
    terminal::{ClipboardMime, ClipboardReadError},
};

static CALLS: AtomicUsize = AtomicUsize::new(0);
static ALWAYS_ZEROED: AtomicBool = AtomicBool::new(true);

fn no_entropy(_: &mut [u8]) -> bool {
    CALLS.fetch_add(1, Ordering::Relaxed);
    false
}

// Not a CSPRNG: only for checking that the registered source is used.
fn counting(buf: &mut [u8]) -> bool {
    let call = CALLS.fetch_add(1, Ordering::Relaxed);
    if buf.iter().any(|&b| b != 0) {
        ALWAYS_ZEROED.store(false, Ordering::Relaxed);
    }
    buf.fill(call as u8);
    true
}

/// Paste into a terminal whose program asked for Kitty paste events, which
/// need entropy for their one-time password. Returns the result and output.
fn paste_event() -> (Result<bool, Error>, Vec<u8>) {
    let output = RefCell::new(Vec::new());
    let mut terminal = Terminal::new(80, 24).unwrap();
    terminal
        .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
        .unwrap()
        .on_clipboard_read(|_term, request| {
            request.reply(Err(ClipboardReadError::Denied), &[], false);
        })
        .unwrap();
    terminal.vt_write(b"\x1b[?5522h");
    let mimes = [ClipboardMime::new("text/plain")];
    let result = terminal.paste(Options::new(), &mimes, |_mime, _out| Ok(()));
    drop(terminal);
    (result, output.into_inner())
}

#[test]
fn registered_source_replaces_the_platform_default() {
    // The platform source works out of the box.
    let (result, platform) = paste_event();
    assert!(result.unwrap());
    assert!(platform.starts_with(b"\x1b]5522;type=read:status=OK:pw="));

    // A source without entropy fails the paste event and writes nothing.
    // SAFETY: This is the only test in this binary, so nothing else uses
    // libghostty concurrently.
    unsafe { set_secure_random_source(Some(no_entropy)) }.unwrap();
    let (result, output) = paste_event();
    assert!(matches!(result, Err(Error::IoError)));
    assert!(output.is_empty());
    assert!(CALLS.load(Ordering::Relaxed) > 0);

    // A working source is used for the password, and always gets a zeroed
    // buffer.
    CALLS.store(0, Ordering::Relaxed);
    // SAFETY: Ditto
    unsafe { set_secure_random_source(Some(counting)) }.unwrap();
    let (result, first) = paste_event();
    assert!(result.unwrap());
    assert!(CALLS.load(Ordering::Relaxed) > 0);
    assert!(ALWAYS_ZEROED.load(Ordering::Relaxed));
    let (_, second) = paste_event();
    assert_ne!(first, second, "each call draws fresh bytes");

    // Clearing the source restores the platform default.
    // SAFETY: Ditto
    unsafe { set_secure_random_source(None) }.unwrap();
    CALLS.store(0, Ordering::Relaxed);
    let (result, _) = paste_event();
    assert!(result.unwrap());
    assert_eq!(CALLS.load(Ordering::Relaxed), 0);
}
