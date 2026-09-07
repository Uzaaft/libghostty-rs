//! Pasting into a terminal, plus terminal-free utilities for validating and
//! encoding paste data.
//!
//! # Pasting into a terminal
//!
//! What a paste writes to the pty depends on the terminal's state, so the
//! recommended way to paste is [`Terminal::paste`]. The embedder hands over the
//! MIME types the clipboard holds (just `text/plain` for an ordinary paste), a
//! reader that produces the data of any one of them, and where the paste came
//! from, and the terminal decides how its current modes apply:
//!
//! - If Kitty clipboard protocol paste events ([`Mode::PASTE_EVENTS`](crate::terminal::Mode::PASTE_EVENTS), mode
//!   5522) are enabled, the paste was user-initiated ([`Source::Clipboard`]),
//!   and a [clipboard read callback](Terminal::on_clipboard_read) is
//!   installed, the terminal sends the program a paste event listing the
//!   clipboard's MIME types with a one-time password instead of the data. The
//!   program then reads what it wants through the clipboard read callback,
//!   which arrives with [`granted`](crate::terminal::ClipboardRead::granted)
//!   set so no permission prompt is needed. No data is read for the event.
//! - Otherwise the first text representation is written: unsafe control bytes
//!   are replaced with spaces, and it is wrapped in bracketed paste sequences
//!   if [`Mode::BRACKETED_PASTE`](crate::terminal::Mode::BRACKETED_PASTE) (mode 2004) is enabled, or has its newlines
//!   converted to carriage returns if not.
//!
//! The data is pulled through the reader only when a representation is
//! actually pasted (so a clipboard holding a large image next to some text
//! costs nothing), and the encoded bytes stream to the
//! [pty write callback](Terminal::on_pty_write) in chunks as they are
//! produced, never in one piece. The callback may be invoked several times for
//! a single paste; the pieces must be written to the pty in order.
//!
//! Text that could inject commands (a newline when unbracketed, or the
//! bracketed paste terminator when bracketed) is refused with
//! [`Error::Rejected`](crate::Error::Rejected) and nothing written unless
//! [`Options::with_allow_unsafe`] is set. The usual flow is to call once,
//! confirm with the user on [`Error::Rejected`](crate::Error::Rejected), and call again with
//! `allow_unsafe` set. Each call reads the text at most once and buffers it
//! whole while the rule is applied, so the source needs no stability across
//! reads (the confirmed retry simply pastes whatever the source holds then)
//! and a refused or failed paste writes nothing at all.
//!
//! ```rust
//! use libghostty_vt::{Error, Terminal, paste::Options, terminal::ClipboardMime};
//! use std::io::Write;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut terminal = Terminal::new(80, 24)?;
//! terminal.on_pty_write(|_term, bytes| {
//!     // Write the bytes to the pty.
//! #   let _ = bytes;
//! })?;
//!
//! let clipboard = "echo hello\n";
//! let mimes = [ClipboardMime::new("text/plain")];
//! let read = |_mime: &str, out: &mut dyn Write| out.write_all(clipboard.as_bytes());
//!
//! match terminal.paste(Options::new(), &mimes, read) {
//!     // Ask the user whether the paste should go ahead, then retry.
//!     Err(Error::Rejected) => {
//!         terminal.paste(Options::new().with_allow_unsafe(true), &mimes, read)?;
//!     }
//!     result => {
//!         result?;
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Building blocks
//!
//! For embedders that encode without a terminal, [`is_safe`] checks if paste
//! data contains potentially dangerous sequences (conservatively, regardless
//! of terminal state) and [`encode`] encodes paste data for writing to the
//! pty, including bracketed paste wrapping and unsafe byte stripping.
//!
//! ## Safety check
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

use std::{ffi::c_void, io::Write};

use crate::{
    Terminal,
    error::{Result, from_result, from_result_with_len},
    ffi,
    terminal::{ClipboardLocation, ClipboardMime},
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

/// Why this paste happened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, int_enum::IntEnum)]
#[repr(i32)]
pub enum Source {
    /// A user action such as a paste keybind, menu item, or middle click.
    #[default]
    Clipboard = ffi::PasteSource::CLIPBOARD,
    /// Programmatic text insertion. This never produces a Kitty paste event.
    Text = ffi::PasteSource::TEXT,
}

/// Options for [`Terminal::paste`].
///
/// This wraps the sized C request directly, so new upstream fields don't need
/// a second Rust representation. The MIME types and the reader are passed to
/// [`Terminal::paste`] instead, since they are only borrowed for that call.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    inner: ffi::Paste,
}

impl Options {
    /// A user-initiated paste from the standard clipboard, refusing text that
    /// could inject commands.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: ffi::Paste {
                location: ClipboardLocation::Standard.into(),
                source: Source::Clipboard.into(),
                ..ffi::sized!(ffi::Paste)
            },
        }
    }

    /// The clipboard the contents came from. Reported to the program on a
    /// paste event (the selection and primary locations are both reported as
    /// the primary selection, the protocol knows only two); no effect on a
    /// text paste.
    #[must_use]
    pub fn with_location(mut self, location: ClipboardLocation) -> Self {
        self.inner.location = location.into();
        self
    }

    /// Why this paste happened.
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.inner.source = source.into();
        self
    }

    /// Write text that could inject commands. Paste with `false`, confirm
    /// with the user on [`Error::Rejected`](crate::Error::Rejected), and paste again with `true`.
    #[must_use]
    pub fn with_allow_unsafe(mut self, allow: bool) -> Self {
        self.inner.allow_unsafe = allow;
        self
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

/// Pasting.
impl Terminal<'_, '_> {
    /// Paste into the terminal according to its current state: a Kitty
    /// clipboard protocol paste event if [`Mode::PASTE_EVENTS`](crate::terminal::Mode::PASTE_EVENTS) is enabled and
    /// a [clipboard read callback](Self::on_clipboard_read) is installed,
    /// otherwise the text framed per [`Mode::BRACKETED_PASTE`](crate::terminal::Mode::BRACKETED_PASTE). See the
    /// [module documentation](crate::paste) for the full behavior. Output
    /// streams through the [pty write callback](Self::on_pty_write) in chunks.
    /// The viewport is not scrolled; that is up to the embedder, as for key
    /// input.
    ///
    /// `mimes` lists the MIME types of the representations available, in
    /// preferred order. A text paste reads and writes the first entry with a
    /// text MIME type such as `text/plain` and ignores the rest. A paste event
    /// lists every entry and reads none.
    ///
    /// `reader` produces the data of a representation by writing it to the
    /// given writer. It is called at most once per paste: for the text
    /// representation being pasted, never for anything else and never for a
    /// paste event. The MIME type requested is always an entry of `mimes`. An
    /// error from the reader fails the paste with [`Error::IoError`](crate::Error::IoError).
    ///
    /// A paste event records a session grant for its one-time password only
    /// once the event is written; a failed call never leaves a grant for an
    /// event that was never sent.
    ///
    /// Returns whether anything was written to the pty (the encoded text or a
    /// paste event). `false` means there was nothing to paste: no non-empty
    /// text representation.
    ///
    /// # Errors
    ///
    /// Errors write nothing to the pty.
    ///
    /// - [`Error::Rejected`](crate::Error::Rejected) if the text could inject commands and
    ///   [`Options::with_allow_unsafe`] is not set.
    /// - [`Error::InvalidValue`](crate::Error::InvalidValue) if no pty write callback is installed.
    /// - [`Error::IoError`](crate::Error::IoError) if the reader failed, or if there is no secure
    ///   entropy source to mint a paste event password.
    /// - [`Error::OutOfMemory`](crate::Error::OutOfMemory).
    pub fn paste<F>(
        &mut self,
        options: Options,
        mimes: &[ClipboardMime<'_>],
        mut reader: F,
    ) -> Result<bool>
    where
        F: FnMut(&str, &mut dyn Write) -> std::io::Result<()>,
    {
        unsafe extern "C" fn read<F>(
            userdata: *mut c_void,
            mime: ffi::String,
            writer: ffi::Writer,
        ) -> bool
        where
            F: FnMut(&str, &mut dyn Write) -> std::io::Result<()>,
        {
            // SAFETY: `userdata` is the reader, which `paste` exclusively
            // borrows for the duration of the call.
            let reader = unsafe { &mut *userdata.cast::<F>() };
            // SAFETY: libghostty passes back an entry of `mimes` exactly as
            // given (the same pointer and length), and every `ClipboardMime`
            // was created from a `&str`.
            let mime = unsafe { mime.to_str() };
            reader(mime, &mut PasteWriter(writer)).is_ok()
        }

        let raw = ffi::Paste {
            // `ClipboardMime` is `repr(transparent)` over `ffi::String`, so
            // the slice can be handed over without copying.
            mimes: mimes.as_ptr().cast(),
            mimes_len: mimes.len(),
            reader: ffi::MimeReader {
                read: Some(read::<F>),
                userdata: (&raw mut reader).cast(),
            },
            ..options.inner
        };
        let mut written = false;
        // SAFETY: The request, the MIME types and the reader all outlive this
        // synchronous call, and the exclusive terminal borrow serializes it.
        from_result(unsafe {
            ffi::ghostty_terminal_paste(self.inner.as_raw(), &raw const raw, &raw mut written)
        })?;
        Ok(written)
    }
}

// Only ever handed to the reader as `&mut dyn Write`, so the libghostty writer
// cannot escape the reader call it is valid for.
struct PasteWriter(ffi::Writer);

impl Write for PasteWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let write = self
            .0
            .write
            .ok_or_else(|| std::io::Error::other("missing paste writer"))?;
        // SAFETY: libghostty's writer is valid for the duration of the reader
        // call, which is the only time a `PasteWriter` exists.
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
    use crate::{
        Error,
        terminal::{ClipboardReadError, Mode},
    };
    use std::cell::{Cell, RefCell};

    const TEXT: [ClipboardMime<'static>; 1] = [ClipboardMime::new("text/plain")];

    /// What a paste wrote to the pty and which MIME types it read.
    ///
    /// The reader only records: a panic inside it would abort the whole test
    /// binary instead of failing the test.
    fn paste(
        terminal: &mut Terminal<'_, '_>,
        output: &RefCell<Vec<u8>>,
        options: Options,
        mimes: &[ClipboardMime<'_>],
        data: &[u8],
    ) -> (Result<bool>, Vec<String>) {
        output.borrow_mut().clear();
        let mut reads = Vec::new();
        let result = terminal.paste(options, mimes, |mime, out| {
            reads.push(mime.to_owned());
            out.write_all(data)
        });
        (result, reads)
    }

    #[test]
    fn text_paste_follows_bracketed_paste_mode() {
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap();
        let mimes = [
            ClipboardMime::new("image/png"),
            ClipboardMime::new("text/plain"),
        ];

        // Unbracketed: unsafe control bytes become spaces. Only the text
        // representation is read.
        let (result, reads) = paste(&mut terminal, &output, Options::new(), &mimes, b"a\0b");
        assert!(result.unwrap());
        assert_eq!(reads, ["text/plain"]);
        assert_eq!(*output.borrow(), b"a b");

        // A newline could inject a command, so it needs confirmation.
        let (result, _) = paste(&mut terminal, &output, Options::new(), &TEXT, b"a\nb");
        assert!(matches!(result, Err(Error::Rejected)));
        assert!(output.borrow().is_empty());
        let options = Options::new().with_allow_unsafe(true);
        let (result, _) = paste(&mut terminal, &output, options, &TEXT, b"a\nb");
        assert!(result.unwrap());
        assert_eq!(*output.borrow(), b"a\rb");

        // Bracketed: newlines are safe, but the end sequence is not.
        terminal.set_mode(Mode::BRACKETED_PASTE, true).unwrap();
        let (result, _) = paste(&mut terminal, &output, Options::new(), &TEXT, b"a\nb");
        assert!(result.unwrap());
        assert_eq!(*output.borrow(), b"\x1b[200~a\nb\x1b[201~");
        let (result, _) = paste(
            &mut terminal,
            &output,
            Options::new(),
            &TEXT,
            b"a\x1b[201~b",
        );
        assert!(matches!(result, Err(Error::Rejected)));
        assert!(output.borrow().is_empty());
    }

    #[test]
    fn nothing_to_paste_and_failures_write_nothing() {
        let output = RefCell::new(Vec::new());
        let mut terminal = Terminal::new(80, 24).unwrap();

        // Without a pty write callback there is nowhere to paste to.
        let (result, reads) = paste(&mut terminal, &output, Options::new(), &TEXT, b"hi");
        assert!(matches!(result, Err(Error::InvalidValue)));
        assert!(reads.is_empty());

        terminal
            .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap();

        // No MIME types, or only non-text ones, is nothing to paste.
        let (result, reads) = paste(&mut terminal, &output, Options::new(), &[], b"");
        assert!(!result.unwrap());
        assert!(reads.is_empty());
        let png = [ClipboardMime::new("image/png")];
        let (result, reads) = paste(&mut terminal, &output, Options::new(), &png, b"png");
        assert!(!result.unwrap());
        assert!(reads.is_empty());
        // An empty text representation is nothing to paste either.
        let (result, _) = paste(&mut terminal, &output, Options::new(), &TEXT, b"");
        assert!(!result.unwrap());

        // A failing reader fails the paste.
        output.borrow_mut().clear();
        let result = terminal.paste(Options::new(), &TEXT, |_mime, out| {
            out.write_all(b"partial")?;
            Err(std::io::Error::other("clipboard went away"))
        });
        assert!(matches!(result, Err(Error::IoError)));
        assert!(output.borrow().is_empty());
    }

    /// Extract the one-time password from a paste event.
    fn event_password(output: &[u8]) -> String {
        let output = std::str::from_utf8(output).unwrap();
        let start = output.find(":pw=").expect("paste event carries a password") + 4;
        let len = output[start..].find(['\x1b', ';', ':']).unwrap();
        output[start..start + len].to_owned()
    }

    #[test]
    fn kitty_paste_events_replace_user_pastes() {
        let output = RefCell::new(Vec::new());
        let granted = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .on_pty_write(|_term, bytes| output.borrow_mut().extend_from_slice(bytes))
            .unwrap();

        // The program enabling mode 5522 alone doesn't produce events.
        terminal.vt_write(b"\x1b[?5522h");
        assert!(terminal.mode(Mode::PASTE_EVENTS).unwrap());
        let (result, _) = paste(&mut terminal, &output, Options::new(), &TEXT, b"hi");
        assert!(result.unwrap());
        assert_eq!(*output.borrow(), b"hi");

        // Once clipboard reads are handled, a user paste becomes an event
        // listing every MIME type, and nothing is read.
        terminal
            .on_clipboard_read(|_term, request| {
                granted.borrow_mut().push(request.granted());
                request.reply(Err(ClipboardReadError::Denied), &[], false);
            })
            .unwrap();
        let mimes = [
            ClipboardMime::new("text/plain"),
            ClipboardMime::new("image/png"),
        ];
        output.borrow_mut().clear();
        let result = terminal.paste(Options::new(), &mimes, |_mime, _out| {
            reads.set(reads.get() + 1);
            Ok(())
        });
        assert!(result.unwrap());
        assert_eq!(reads.get(), 0);
        let event = output.borrow().clone();
        let pw = event_password(&event);
        assert_eq!(
            event,
            format!(
                concat!(
                    "\x1b]5522;type=read:status=OK:pw={pw}\x1b\\",
                    // "text/plain image/png\n"
                    "\x1b]5522;type=read:status=DATA:mime=Lg==:pw={pw};dGV4dC9wbGFpbiBpbWFnZS9wbmcK\x1b\\",
                    "\x1b]5522;type=read:status=DONE:pw={pw}\x1b\\",
                ),
                pw = pw
            )
            .into_bytes()
        );

        // The program's follow-up read with that password is already granted.
        // Per the spec, a password only counts together with a name ("app").
        terminal.vt_write(
            format!("\x1b]5522;type=read:id=r:name=YXBw:pw={pw};dGV4dC9wbGFpbg==\x1b\\").as_bytes(),
        );
        assert_eq!(*granted.borrow(), [true]);

        // Other locations are reported as the primary selection.
        let options = Options::new().with_location(ClipboardLocation::Selection);
        let (result, _) = paste(&mut terminal, &output, options, &TEXT, b"hi");
        assert!(result.unwrap());
        assert!(
            output
                .borrow()
                .starts_with(b"\x1b]5522;type=read:status=OK:loc=primary:")
        );

        // Programmatic insertion always pastes the text.
        let options = Options::new().with_source(Source::Text);
        let (result, reads) = paste(&mut terminal, &output, options, &TEXT, b"hi");
        assert!(result.unwrap());
        assert_eq!(reads, ["text/plain"]);
        assert_eq!(*output.borrow(), b"hi");
    }
}
