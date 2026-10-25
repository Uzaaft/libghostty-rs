//! Rust port of upstream's `example/wasm-vt` terminal demo.

use libghostty_vt::{
    Terminal,
    alloc::Bytes,
    error::Result,
    fmt::{Formatter, FormatterOptions},
};

/// Create a terminal, feed it VT data, and return its contents as plain text.
pub fn format_terminal(cols: u16, rows: u16, input: &[u8]) -> Result<Bytes<'static>> {
    let mut terminal = Terminal::new(cols, rows)?;
    terminal.vt_write(input);
    let mut formatter = Formatter::new(&terminal, FormatterOptions::new().with_trim(true))?;
    formatter.format_alloc(None)
}

// This small host bridge belongs to the example, not the library API. JavaScript
// only transfers bytes; Rust owns the terminal, formatter and output buffer.
#[cfg(target_family = "wasm")]
mod browser {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        // The host must copy these bytes before returning, without calling back
        // into Wasm. The Rust buffer is dropped immediately after this call.
        fn display(ptr: *const u8, len: usize, error: bool);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn input_alloc(len: u32) -> *mut u8 {
        Box::into_raw(vec![0u8; len as usize].into_boxed_slice()).cast()
    }

    /// # Safety
    /// `ptr` and `len` must match a live allocation returned by `input_alloc`.
    /// The host must not use it after this call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn input_free(ptr: *mut u8, len: u32) {
        // SAFETY: The host returns the exact allocation and length.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len as usize)) });
    }

    /// # Safety
    /// `ptr` and `len` must match a live `input_alloc` allocation. The host must
    /// not change or free it until this call returns, including in `display`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn run(cols: u16, rows: u16, ptr: *const u8, len: u32) {
        // SAFETY: The host keeps the input allocation alive for this call.
        let input = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        match super::format_terminal(cols, rows, input) {
            Ok(output) => {
                // SAFETY: The host copies the output synchronously.
                unsafe { display(output.as_ptr(), output.len(), false) };
            }
            Err(error) => {
                let message = error.to_string();
                // SAFETY: Ditto.
                unsafe { display(message.as_ptr(), message.len(), true) };
            }
        }
    }
}
