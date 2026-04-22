//! Pure-Rust FFI shims for Miri.
//!
//! When running under `cargo miri test`, the native Ghostty shared library
//! cannot be loaded. This module provides `#[unsafe(no_mangle)] extern "C"`
//! implementations of the FFI symbols so that Miri can execute the full
//! Rust wrapper code paths — including the unsafe seams — without crossing
//! into C/Zig.
//!
//! The shims replicate the *ABI contract* of each function, not the full
//! terminal semantics. They allocate real backing storage behind the opaque
//! handles so pointer provenance and layout discipline are exercised under
//! Miri, but they only implement enough state to cover the wrapper test
//! scenarios.
//!
//! ## Adding new shims
//!
//! Each subsystem (OSC, key, mouse, …) lives in its own sub-module. When
//! adding a new shim:
//!
//! 1. Define a concrete backing struct that carries the state the wrapper
//!    tests need.
//! 2. Implement the `extern "C"` functions using `Box` allocation behind
//!    the opaque pointer (cast to/from the `*Impl` type).
//! 3. Make sure `_free` drops the `Box`.

use std::os::raw::{c_char, c_void};

use crate::bindings::{self, Allocator, Result as GhosttyResult};

// ---------------------------------------------------------------------------
// OSC parser shims
// ---------------------------------------------------------------------------

/// Backing storage for a shimmed OSC parser. The real parser is a complex
/// state machine; this shim only needs to accumulate the raw byte stream
/// so that `osc_end` can produce a command with the right type and data.
struct OscParserState {
    /// Raw bytes fed via `ghostty_osc_next`.
    bytes: Vec<u8>,
}

/// Backing storage for a shimmed OSC command produced by `osc_end`.
struct OscCommandState {
    command_type: bindings::OscCommandType::Type,
    /// Null-terminated title string, kept alive for the lifetime of the
    /// command so that `ghostty_osc_command_data` can hand out a pointer.
    title: Option<Vec<u8>>,
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_new(
    _allocator: *const Allocator,
    parser: *mut bindings::OscParser,
) -> GhosttyResult::Type {
    let state = Box::new(OscParserState { bytes: Vec::new() });
    // Cast the heap allocation to the opaque `*mut OscParserImpl` handle.
    unsafe { *parser = Box::into_raw(state).cast() };
    GhosttyResult::SUCCESS
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_free(parser: bindings::OscParser) {
    if !parser.is_null() {
        drop(unsafe { Box::from_raw(parser.cast::<OscParserState>()) });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_reset(parser: bindings::OscParser) {
    let state = unsafe { &mut *parser.cast::<OscParserState>() };
    state.bytes.clear();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_next(parser: bindings::OscParser, byte: u8) {
    let state = unsafe { &mut *parser.cast::<OscParserState>() };
    state.bytes.push(byte);
}

/// Minimal parser: recognises `2;...` as a title-change command.
#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_end(
    parser: bindings::OscParser,
    _terminator: u8,
) -> bindings::OscCommand {
    let state = unsafe { &mut *parser.cast::<OscParserState>() };
    let bytes = &state.bytes;

    let (cmd_type, title) = if bytes.starts_with(b"2;") {
        // Extract the title payload and null-terminate it (the real C
        // implementation hands back a `const char*`).
        let payload = &bytes[2..];
        let mut buf = Vec::with_capacity(payload.len() + 1);
        buf.extend_from_slice(payload);
        buf.push(0); // null terminator
        (bindings::OscCommandType::CHANGE_WINDOW_TITLE, Some(buf))
    } else {
        (bindings::OscCommandType::INVALID, None)
    };

    let cmd = Box::new(OscCommandState {
        command_type: cmd_type,
        title,
    });
    Box::into_raw(cmd).cast()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_command_type(
    command: bindings::OscCommand,
) -> bindings::OscCommandType::Type {
    let cmd = unsafe { &*command.cast::<OscCommandState>() };
    cmd.command_type
}

/// Writes the requested data field into `out`.
///
/// For `CHANGE_WINDOW_TITLE_STR` the real C API writes a single
/// `const char*` pointer into the output slot. The Rust wrapper's generic
/// `get<T>` path calls this with `T = &str` and a `MaybeUninit<&str>`
/// destination — which is wider than a single pointer on all platforms.
/// This ABI mismatch is the exact bug the shim is designed to surface
/// under Miri.
#[unsafe(no_mangle)]
unsafe extern "C" fn ghostty_osc_command_data(
    command: bindings::OscCommand,
    data: bindings::OscCommandData::Type,
    out: *mut c_void,
) -> bool {
    let cmd = unsafe { &*command.cast::<OscCommandState>() };

    match data {
        bindings::OscCommandData::CHANGE_WINDOW_TITLE_STR => {
            if let Some(ref title_buf) = cmd.title {
                // Write exactly what the real C side writes: a single
                // `const char*` pointer, not a fat Rust `&str`.
                let ptr: *const c_char = title_buf.as_ptr().cast();
                unsafe { out.cast::<*const c_char>().write(ptr) };
                true
            } else {
                false
            }
        }
        _ => false,
    }
}
