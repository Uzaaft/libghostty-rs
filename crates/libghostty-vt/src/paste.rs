//! Utilities for validating paste data safety.
//!
//! # Example
//!
//! ## Safety Check
//!
//! ```rust
//! use libghostty_vt::paste;
//!
//! let safe_data = "hello world";
//! let unsafe_data = "rm -rf /\n";
//!
//! if paste::is_safe(safe_data) {
//!     println!("Safe to paste");
//! }
//!
//! if !paste::is_safe(unsafe_data) {
//!     println!("Unsafe! Contains newline");
//! }
//! ```
//!
//! ## Encoding
//!
//! ```rust
//! use libghostty_vt::paste;
//!
//! let mut data = *b"hello\nworld";
//! let mut buf = [0u8; 64];
//!
//! if let Ok(len) = paste::encode(&mut data, true, &mut buf) {
//!     println!("Encoded {len} bytes: {}", buf[..len].escape_ascii());
//! }
//! ```

use crate::{
    error::{Result, from_result_with_len},
    ffi,
};

/// Check if paste data is safe to paste into the terminal.
///
/// Data is considered unsafe if it contains:
///   * Newlines (`\n`) which can inject commands
///   * The bracketed paste end sequence (`\x1b[201~`) which can be used to exit bracketed paste
///     mode and inject commands
///
/// This check is conservative and considers data unsafe regardless of current terminal state.
#[must_use]
pub fn is_safe(data: &str) -> bool {
    unsafe { ffi::ghostty_paste_is_safe(data.as_ptr().cast(), data.len()) }
}

/// Encode paste data for writing to the terminal pty.
///
/// This function prepares paste data for terminal input by:
///
/// - Stripping unsafe control bytes (NUL, ESC, DEL, etc.) by replacing them
///   with spaces
/// - Wrapping the data in bracketed paste sequences if `bracketed` is true
/// - Replacing newlines with carriage returns if `bracketed` is false
///
/// The input `data` buffer is modified in place during encoding. The encoded
/// result (potentially with bracketed paste prefix/suffix) is written to the
/// output buffer.
///
/// If the output buffer is too small, the function returns
/// `Err(Error::OutOfSpace { required })` where `required` is the required
/// The caller can then retry with a sufficiently sized buffer.
pub fn encode(data: &mut [u8], bracketed: bool, buf: &mut [u8]) -> Result<usize> {
    let mut written = 0usize;
    let result = unsafe {
        ffi::ghostty_paste_encode(
            data.as_mut_ptr().cast(),
            data.len(),
            bracketed,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &raw mut written,
        )
    };
    from_result_with_len(result, written)
}

/// Why text is being pasted into the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum Source {
    /// A user action such as a paste keybind, menu item, or middle click.
    Clipboard = ffi::PasteSource::CLIPBOARD,
    /// Programmatic text insertion; never produces a Kitty paste event.
    Text = ffi::PasteSource::TEXT,
}

/// Options applied using the terminal's current paste modes.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Clipboard location reported in Kitty paste events.
    pub location: crate::terminal::ClipboardLocation,
    /// Whether this is a clipboard action or programmatic insertion.
    pub source: Source,
    /// Permit text otherwise rejected as unsafe. Set only after host confirmation.
    pub allow_unsafe: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            location: crate::terminal::ClipboardLocation::Standard,
            source: Source::Clipboard,
            allow_unsafe: false,
        }
    }
}

impl crate::Terminal<'_, '_> {
    /// Paste MIME data according to terminal modes, sending output to the PTY handler.
    ///
    /// `mimes` lists available representations in preferred order. `reader` is
    /// called at most once for the chosen representation and writes it to the
    /// supplied sink. Kitty paste events list MIME types without calling it.
    /// Returns false if there is nothing to paste. [`crate::Error::Rejected`]
    /// leaves PTY output untouched so the host can confirm and retry with
    /// `allow_unsafe`. A reader failure also writes nothing to the PTY.
    pub fn paste<F>(&mut self, options: Options, mimes: &[&str], mut reader: F) -> Result<bool>
    where
        F: FnMut(&str, &mut dyn std::io::Write) -> std::io::Result<()>,
    {
        unsafe extern "C" fn read<F>(
            userdata: *mut std::ffi::c_void,
            mime: ffi::String,
            writer: ffi::Writer,
        ) -> bool
        where
            F: FnMut(&str, &mut dyn std::io::Write) -> std::io::Result<()>,
        {
            // SAFETY: Ghostty passes back an entry from the UTF-8 mimes array,
            // and the closure remains exclusively borrowed during this call.
            let reader = unsafe { &mut *userdata.cast::<F>() };
            let mime = unsafe { mime.to_str() };
            reader(mime, &mut PasteWriter(writer)).is_ok()
        }
        let mimes: Vec<ffi::String> = mimes.iter().map(|mime| (*mime).into()).collect();
        let raw = ffi::Paste {
            location: options.location.into(),
            source: options.source.into(),
            allow_unsafe: options.allow_unsafe,
            mimes: mimes.as_ptr(),
            mimes_len: mimes.len(),
            reader: ffi::MimeReader {
                read: Some(read::<F>),
                userdata: std::ptr::from_mut(&mut reader).cast(),
            },
            ..ffi::sized!(ffi::Paste)
        };
        let mut written = false;
        // All request pointers and callbacks remain live for this synchronous call.
        crate::error::from_result(unsafe {
            ffi::ghostty_terminal_paste(self.inner.as_raw(), &raw, &mut written)
        })?;
        Ok(written)
    }
}

// Kept private and borrowed only inside the MIME reader callback: the Ghostty
// writer cannot escape into user state or outlive the current paste call.
struct PasteWriter(ffi::Writer);
impl std::io::Write for PasteWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let write = self
            .0
            .write
            .ok_or_else(|| std::io::Error::other("missing paste writer"))?;
        if unsafe { write(self.0.userdata, bytes.as_ptr(), bytes.len()) } {
            Ok(bytes.len())
        } else {
            Err(std::io::Error::other("paste writer rejected output"))
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;
    use crate::{Error, Terminal, terminal::Mode};
    use std::cell::RefCell;

    #[test]
    fn terminal_paste_rejects_then_encodes_with_current_modes() {
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .on_pty_write(|_, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap();
        let result = terminal.paste(Options::default(), &["text/plain"], |mime, writer| {
            assert_eq!(mime, "text/plain");
            writer.write_all(b"a\nb")
        });
        assert!(matches!(result, Err(Error::Rejected)));
        assert!(output.borrow().is_empty());
        assert!(
            terminal
                .paste(
                    Options {
                        allow_unsafe: true,
                        ..Options::default()
                    },
                    &["text/plain"],
                    |_, writer| writer.write_all(b"a\nb")
                )
                .unwrap()
        );
        assert_eq!(&*output.borrow(), b"a\rb");
        output.borrow_mut().clear();
        terminal.set_mode(Mode::BRACKETED_PASTE, true).unwrap();
        terminal
            .paste(Options::default(), &["text/plain"], |_, writer| {
                writer.write_all(b"a\nb")
            })
            .unwrap();
        assert_eq!(&*output.borrow(), b"\x1b[200~a\nb\x1b[201~");
        output.borrow_mut().clear();
        assert!(matches!(
            terminal.paste(Options::default(), &["text/plain"], |_, _| Err(
                std::io::Error::other("read failed")
            )),
            Err(Error::IoError)
        ));
        assert!(output.borrow().is_empty());
        assert!(
            !terminal
                .paste(Options::default(), &[], |_, _| panic!(
                    "empty paste must not read"
                ))
                .unwrap()
        );
    }
}
