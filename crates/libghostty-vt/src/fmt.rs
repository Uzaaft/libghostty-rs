//! Format terminal content as plain text, VT sequences, or HTML.
//!
//! A formatter captures a reference to a terminal and formatting options.
//! It can be used repeatedly to produce output that reflects the current
//! terminal state at the time of each format call.
use std::{marker::PhantomData, mem::MaybeUninit};

use crate::{
    alloc::{Allocator, Bytes, Object},
    error::{Error, Result, from_optional_result_uninit, from_result, from_result_with_len},
    ffi::{self, FormatterOption as Opt},
    selection::Selection,
    terminal::Terminal,
};

/// Formatter that formats terminal content.
#[derive(Debug)]
pub struct Formatter<'t, 'alloc: 'cb, 'cb: 't> {
    inner: Object<'alloc, ffi::FormatterImpl>,
    _terminal: PhantomData<&'t Terminal<'alloc, 'cb>>,
}

bitflags::bitflags! {
    /// Terminal state to add to VT output. Plain text and HTML ignore these bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Extras: ffi::FormatterExtras {
        /// Restore the color palette with OSC 4.
        const PALETTE = ffi::FORMATTER_EXTRA_PALETTE;
        /// Restore terminal modes that differ from their defaults.
        const MODES = ffi::FORMATTER_EXTRA_MODES;
        /// Restore the scrolling region with DECSTBM and DECSLRM.
        const SCROLLING_REGION = ffi::FORMATTER_EXTRA_SCROLLING_REGION;
        /// Restore tab stops by clearing them all and setting each one again.
        const TABSTOPS = ffi::FORMATTER_EXTRA_TABSTOPS;
        /// Restore the working directory with OSC 7.
        const PWD = ffi::FORMATTER_EXTRA_PWD;
        /// Restore keyboard modes such as modifyOtherKeys.
        const KEYBOARD = ffi::FORMATTER_EXTRA_KEYBOARD;
        /// Restore the cursor position with CUP.
        const CURSOR = ffi::FORMATTER_EXTRA_CURSOR;
        /// Restore the style the cursor writes new text with, using SGR.
        const STYLE = ffi::FORMATTER_EXTRA_STYLE;
        /// Restore the hyperlink the cursor writes new text with, using OSC 8.
        const HYPERLINK = ffi::FORMATTER_EXTRA_HYPERLINK;
        /// Restore character protection with DECSCA.
        const PROTECTION = ffi::FORMATTER_EXTRA_PROTECTION;
        /// Restore the Kitty keyboard protocol flags.
        const KITTY_KEYBOARD = ffi::FORMATTER_EXTRA_KITTY_KEYBOARD;
        /// Restore the character sets, such as the line drawing set.
        const CHARSETS = ffi::FORMATTER_EXTRA_CHARSETS;
        /// No extras. VT output has only the text and its styles.
        const NONE = ffi::FORMATTER_EXTRAS_NONE;
        /// Every extra.
        const ALL = ffi::FORMATTER_EXTRAS_ALL;
        /// The palette, cursor style and cursor hyperlink.
        const DEFAULT = ffi::FORMATTER_EXTRAS_DEFAULT;
    }
}

impl Default for Extras {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl<'t, 'alloc: 'cb, 'cb: 't> Formatter<'t, 'alloc, 'cb> {
    /// Create a formatter for a terminal's active screen.
    ///
    /// A new formatter emits plain text with trailing whitespace trimmed,
    /// without unwrapping, and with [`Extras::DEFAULT`].
    pub fn new(terminal: &'t Terminal<'alloc, 'cb>) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), terminal) }
    }

    /// Create a formatter for a terminal's active screen.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        terminal: &'t Terminal<'alloc, 'cb>,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), terminal) }
    }

    /// Create a formatter restricted to the terminal's current active selection.
    ///
    /// Returns `Ok(None)` when there is no active selection, rather than
    /// creating a formatter for the whole screen. The formatter copies the
    /// selection at creation; later selection changes do not change its range.
    /// All other options have the same defaults as [`Self::new`]. For clipboard
    /// text, use plain output with unwrap and trim both true.
    ///
    /// ```
    /// use libghostty_vt::{Terminal, fmt::{Format, Formatter}};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let terminal = Terminal::new(80, 24)?;
    /// if let Some(mut formatter) = Formatter::new_selection(&terminal)? {
    ///     formatter.set_format(Format::Plain)?.set_unwrap(true)?.set_trim(true)?;
    ///     let selected = formatter.format_alloc(None)?;
    ///     // Copy only `selected` to the clipboard.
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The terminal cannot change screens or invalidate the selection while
    /// the formatter is in use:
    ///
    /// ```compile_fail,E0502
    /// use libghostty_vt::{Terminal, fmt::Formatter};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut terminal = Terminal::new(10, 3)?;
    /// let mut formatter = Formatter::new_selection(&terminal)?.unwrap();
    /// terminal.vt_write(b"\x1b[?1049h");
    /// formatter.format_alloc(None)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new_selection(terminal: &'t Terminal<'alloc, 'cb>) -> Result<Option<Self>> {
        // SAFETY: A NULL allocator is always valid.
        unsafe { Self::new_selection_inner(std::ptr::null(), terminal) }
    }

    /// Create a formatter restricted to the terminal's current active selection.
    ///
    /// Like [`Self::new_selection`], returns `Ok(None)` when there is no active
    /// selection. See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_selection_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        terminal: &'t Terminal<'alloc, 'cb>,
    ) -> Result<Option<Self>> {
        // SAFETY: Borrow checking should forbid invalid allocators.
        unsafe { Self::new_selection_inner(alloc.to_raw(), terminal) }
    }

    /// # Safety
    ///
    /// `alloc` must be NULL or point to an allocator valid for `'alloc`.
    unsafe fn new_selection_inner(
        alloc: *const ffi::Allocator,
        terminal: &'t Terminal<'alloc, 'cb>,
    ) -> Result<Option<Self>> {
        let Some(selection) = terminal.selection()? else {
            return Ok(None);
        };
        // SAFETY: Upheld by caller.
        let mut formatter = unsafe { Self::new_inner(alloc, terminal) }?;
        // SAFETY: The snapshot comes from this very terminal's active screen.
        // The formatter's shared terminal borrow prevents writes, resize, reset,
        // and screen switches for its lifetime. Shared selection/gesture,
        // tracked-ref, and render-state operations do not invalidate grid refs.
        // Upstream copies the selection, not the pointer to this local value.
        unsafe { formatter.set_selection(Some(&selection)) }?;
        Ok(Some(formatter))
    }

    /// # Safety
    ///
    /// `alloc` must be NULL or point to an allocator valid for `'alloc`.
    unsafe fn new_inner(
        alloc: *const ffi::Allocator,
        terminal: &'t Terminal<'alloc, 'cb>,
    ) -> Result<Self> {
        let mut raw: ffi::Formatter = std::ptr::null_mut();

        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(alloc, &raw mut raw, terminal.inner.as_raw())
        };
        from_result(result)?;

        Ok(Self {
            inner: Object::new(raw)?,
            _terminal: PhantomData,
        })
    }

    fn get_optional<T>(&self, tag: Opt::Type) -> Result<Option<T>> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe {
            ffi::ghostty_formatter_get(self.inner.as_raw(), tag, value.as_mut_ptr().cast())
        };
        from_optional_result_uninit(result, value)
    }

    fn get<T>(&self, tag: Opt::Type) -> Result<T> {
        self.get_optional(tag)?.ok_or(Error::InvalidValue)
    }

    fn set_optional<T>(&mut self, tag: Opt::Type, value: Option<&T>) -> Result<&mut Self> {
        let value = value.map_or(std::ptr::null(), |v| std::ptr::from_ref(v).cast());
        from_result(unsafe { ffi::ghostty_formatter_set(self.inner.as_raw(), tag, value) })?;
        Ok(self)
    }

    /// The output format: plain text, VT sequences, or HTML.
    pub fn format(&self) -> Result<Format> {
        self.get::<ffi::FormatterFormat::Type>(Opt::EMIT)?
            .try_into()
            .map_err(|_| Error::InvalidValue)
    }

    /// Set the output format. Defaults to plain text.
    pub fn set_format(&mut self, value: Format) -> Result<&mut Self> {
        self.set_optional(Opt::EMIT, Some(&ffi::FormatterFormat::Type::from(value)))
    }

    /// Whether to join lines that wrapped because they were too long for the screen.
    pub fn unwrap(&self) -> Result<bool> {
        self.get(Opt::UNWRAP)
    }

    /// Join lines that wrapped because they were too long for the screen.
    /// Defaults to false.
    pub fn set_unwrap(&mut self, value: bool) -> Result<&mut Self> {
        self.set_optional(Opt::UNWRAP, Some(&value))
    }

    /// Whether to remove spaces from the end of each line that has other text.
    pub fn trim(&self) -> Result<bool> {
        self.get(Opt::TRIM)
    }

    /// Remove spaces from the end of each line that has other text.
    ///
    /// Blank lines at the end of the output are always removed. The one
    /// exception is VT output with trimming turned off, which keeps them so
    /// the output fills the same rows as the screen. Defaults to true.
    pub fn set_trim(&mut self, value: bool) -> Result<&mut Self> {
        self.set_optional(Opt::TRIM, Some(&value))
    }

    /// The selection restricting output, or `None` for the whole screen.
    /// The returned snapshot is valid until the next mutating terminal call.
    pub fn selection(&self) -> Result<Option<Selection<'t>>> {
        self.get_optional::<ffi::Selection>(Opt::SELECTION)
            // SAFETY: set_selection requires snapshots from the borrowed terminal.
            .map(|v| v.map(|raw| unsafe { Selection::from_raw(raw) }))
    }

    /// Format only the content inside a selection.
    ///
    /// The formatter keeps its own copy of the selection. The selection must
    /// be a valid snapshot for this formatter's terminal. Pass `None` to clear
    /// it and format the whole screen again.
    ///
    /// To format the terminal's active selection safely, use
    /// [`Self::new_selection`]. For clipboard text, use plain output with
    /// unwrap and trim both true.
    ///
    /// # Safety
    ///
    /// Both endpoints must be valid snapshots from this formatter's terminal
    /// and its currently active screen. The lifetime prevents stale snapshots,
    /// but does not prove they came from the same terminal or screen. Upstream
    /// does not check this. Passing `None` is always safe.
    ///
    /// A selection from another terminal cannot be installed by safe code:
    ///
    /// ```compile_fail,E0133
    /// use libghostty_vt::{Terminal, fmt::Formatter};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let terminal = Terminal::new(10, 3)?;
    /// let other = Terminal::new(10, 3)?;
    /// let selection = other.select_all()?;
    /// let mut formatter = Formatter::new(&terminal)?;
    /// formatter.set_selection(selection.as_ref())?;
    /// # Ok(())
    /// # }
    /// ```
    pub unsafe fn set_selection(&mut self, value: Option<&Selection<'t>>) -> Result<&mut Self> {
        self.set_optional(Opt::SELECTION, value.map(|v| &v.inner))
    }

    /// Terminal state to add to VT output.
    pub fn extras(&self) -> Result<Extras> {
        self.get(Opt::EXTRAS).map(Extras::from_bits_retain)
    }

    /// Set terminal state to add to VT output. Plain text and HTML ignore it.
    pub fn set_extras(&mut self, value: Extras) -> Result<&mut Self> {
        self.set_optional(Opt::EXTRAS, Some(&value.bits()))
    }

    /// Run the formatter and stream output to a writer.
    ///
    /// Each call formats the current terminal state and invokes the writer
    /// synchronously as output becomes available. The writer may be called
    /// more than once.
    ///
    /// If an error occurs, the writer may already contain a partial formatted
    /// output. The operation cannot be resumed from that partial output. This
    /// function does not flush or make the writer's destination durable.
    ///
    /// The writer runs inside a call into libghostty, so a panic in it aborts
    /// the process.
    ///
    /// # Safety
    ///
    /// The writer must not call formatter or terminal APIs using this
    /// formatter or its terminal. The borrow checker can't rule that out,
    /// since the formatter only borrows its terminal immutably, and some
    /// terminal methods that take `&self` still change libghostty state (e.g.
    /// [`Terminal::track_grid_ref`](crate::Terminal::track_grid_ref)).
    ///
    /// # Errors
    ///
    /// Returns [`Error::IoError`] if the writer rejects output, and
    /// [`Error::LimitExceeded`] if output accounting overflows.
    pub unsafe fn format_write<W: std::io::Write>(&mut self, writer: &mut W) -> Result<()> {
        let writer = crate::io::to_writer(writer);
        // SAFETY: The writer outlives this synchronous call, and the caller
        // promises it doesn't reach this formatter or its terminal.
        from_result(unsafe { ffi::ghostty_formatter_format(self.inner.as_raw(), writer) })
    }

    /// Run the formatter and return an allocated buffer with the output.
    ///
    /// Each call formats the current terminal state. The buffer is allocated
    /// using the provided allocator (or the default allocator if `None`).
    /// Empty output returns an empty buffer.
    pub fn format_alloc<'a, 'ctx: 'a>(
        &mut self,
        alloc: Option<&'a Allocator<'ctx>>,
    ) -> Result<Bytes<'a>> {
        let alloc = if let Some(alloc) = alloc {
            alloc.to_raw()
        } else {
            std::ptr::null()
        };

        let mut bytes = std::ptr::null_mut();
        let mut len = 0usize;
        let result = unsafe {
            ffi::ghostty_formatter_format_alloc(
                self.inner.as_raw(),
                alloc,
                std::ptr::from_mut(&mut bytes),
                std::ptr::from_mut(&mut len),
            )
        };
        from_result(result)?;

        // SAFETY: On success, libghostty hands over `len` bytes allocated
        // with `alloc`, or NULL for empty output.
        Ok(unsafe { Bytes::from_raw_parts(bytes, len, alloc) })
    }

    /// Run the formatter and produce output into the caller-provided buffer.
    ///
    /// Each call formats the current terminal state. If the buffer is too small,
    /// returns `Err(Error::OutOfSpace { required })` where `required` is the
    /// required size. The caller can then retry with a larger buffer.
    pub fn format_buf(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut len = 0usize;
        let result = unsafe {
            ffi::ghostty_formatter_format_buf(
                self.inner.as_raw(),
                std::ptr::from_mut(buf).cast(),
                buf.len(),
                std::ptr::from_mut(&mut len),
            )
        };
        // On OUT_OF_SPACE upstream stores the required size in `len`, which
        // the docs above promise callers; plain `from_result` would drop it.
        from_result_with_len(result, len)
    }

    /// Query the required buffer size for the formatted output.
    ///
    /// The result can be used to create a sufficiently large buffer
    /// for [`Formatter::format_buf`].
    pub fn format_len(&mut self) -> Result<usize> {
        let mut len = 0usize;
        let result = unsafe {
            ffi::ghostty_formatter_format_buf(
                self.inner.as_raw(),
                std::ptr::null_mut(),
                0,
                std::ptr::from_mut(&mut len),
            )
        };
        // Empty output succeeds with zero bytes; nonempty output needs space.
        match from_result(result) {
            Ok(()) | Err(Error::OutOfSpace { .. }) => Ok(len),
            Err(e) => Err(e),
        }
    }
}

impl Drop for Formatter<'_, '_, '_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.inner.as_raw()) }
    }
}

/// Output format.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
pub enum Format {
    /// Plain text (no escape sequences).
    Plain = ffi::FormatterFormat::PLAIN,
    /// VT sequences preserving colors, styles, URLs, etc.
    Vt = ffi::FormatterFormat::VT,
    /// HTML with inline styles.
    Html = ffi::FormatterFormat::HTML,
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    /// A writer that accepts at most one byte per write, counting calls.
    struct ByteAtATime {
        bytes: Vec<u8>,
        writes: usize,
    }

    impl std::io::Write for ByteAtATime {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(&buf[..1]);
            Ok(1)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A writer that rejects everything.
    struct Reject;

    impl std::io::Write for Reject {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("rejected"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn format_write<W: std::io::Write>(
        formatter: &mut Formatter<'_, '_, '_>,
        writer: &mut W,
    ) -> Result<()> {
        // SAFETY: None of the test writers touch the terminal.
        unsafe { formatter.format_write(writer) }
    }

    fn format_buf(formatter: &mut Formatter<'_, '_, '_>) -> Vec<u8> {
        let mut buf = vec![0; formatter.format_len().unwrap()];
        let len = formatter.format_buf(&mut buf).unwrap();
        buf.truncate(len);
        buf
    }

    #[test]
    fn options_round_trip_and_invalid_extras_leave_state_unchanged() {
        let terminal = Terminal::new(10, 3).unwrap();
        let mut formatter = Formatter::new(&terminal).unwrap();
        assert_eq!(formatter.format().unwrap(), Format::Plain);
        assert!(formatter.trim().unwrap());
        assert!(!formatter.unwrap().unwrap());
        assert_eq!(formatter.extras().unwrap(), Extras::DEFAULT);
        assert!(formatter.selection().unwrap().is_none());

        formatter
            .set_format(Format::Html)
            .unwrap()
            .set_trim(false)
            .unwrap()
            .set_unwrap(true)
            .unwrap()
            .set_extras(Extras::ALL)
            .unwrap();
        assert_eq!(formatter.format().unwrap(), Format::Html);
        assert!(!formatter.trim().unwrap());
        assert!(formatter.unwrap().unwrap());
        assert_eq!(formatter.extras().unwrap(), Extras::ALL);
        assert!(matches!(
            formatter.set_extras(Extras::from_bits_retain(1 << 31)),
            Err(Error::InvalidValue)
        ));
        assert_eq!(formatter.extras().unwrap(), Extras::ALL);
    }

    #[test]
    fn selection_is_copied_and_can_be_cleared() {
        use crate::terminal::{Point, PointCoordinate};

        let mut terminal = Terminal::new(10, 3).unwrap();
        terminal.vt_write(b"hello world");
        let mut formatter = Formatter::new(&terminal).unwrap();
        {
            let start = terminal
                .grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
                .unwrap();
            let end = terminal
                .grid_ref(Point::Active(PointCoordinate { x: 4, y: 0 }))
                .unwrap();
            let selection = Selection::new(start, end, false);
            // SAFETY: Both endpoints come from the formatter's active screen.
            unsafe { formatter.set_selection(Some(&selection)) }.unwrap();
            assert!(
                formatter
                    .selection()
                    .unwrap()
                    .unwrap()
                    .equals(&terminal, &selection)
                    .unwrap()
            );
        }
        assert_eq!(format_buf(&mut formatter), b"hello");
        // SAFETY: Clearing a selection has no preconditions.
        unsafe { formatter.set_selection(None) }
            .unwrap()
            .set_unwrap(true)
            .unwrap();
        assert!(formatter.selection().unwrap().is_none());
        assert_eq!(format_buf(&mut formatter), b"hello world");
    }

    #[test]
    fn current_selection_formats_only_selected_text_and_keeps_its_snapshot() {
        use crate::terminal::{Point, PointCoordinate};

        let mut terminal = Terminal::new(20, 3).unwrap();
        terminal.vt_write(b"left chosen right\r\nnot selected");
        let at = |x| {
            terminal
                .grid_ref(Point::Active(PointCoordinate { x, y: 0 }))
                .unwrap()
        };
        let selection = Selection::new(at(5), at(10), false);
        terminal.set_selection(Some(&selection)).unwrap();

        let mut formatter = Formatter::new_selection(&terminal).unwrap().unwrap();
        let mut allocated = Formatter::new_selection_with_alloc(&Allocator::GLOBAL, &terminal)
            .unwrap()
            .unwrap();
        formatter
            .set_format(Format::Plain)
            .unwrap()
            .set_unwrap(true)
            .unwrap()
            .set_trim(true)
            .unwrap();
        // Clearing the terminal-owned selection does not invalidate the copy.
        terminal.set_selection(None).unwrap();
        let _tracked = terminal
            .track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
            .unwrap();
        assert_eq!(&*formatter.format_alloc(None).unwrap(), b"chosen");
        assert_eq!(
            &*allocated.format_alloc(Some(&Allocator::GLOBAL)).unwrap(),
            b"chosen"
        );
    }

    #[test]
    fn current_selection_absence_does_not_format_the_whole_screen() {
        let mut terminal = Terminal::new(20, 3).unwrap();
        terminal.vt_write(b"not clipboard text");
        assert!(Formatter::new_selection(&terminal).unwrap().is_none());
        assert!(
            Formatter::new_selection_with_alloc(&Allocator::GLOBAL, &terminal)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            &*Formatter::new(&terminal)
                .unwrap()
                .format_alloc(None)
                .unwrap(),
            b"not clipboard text"
        );
    }

    #[test]
    fn empty_output_needs_no_buffer() {
        let terminal = Terminal::new(10, 3).unwrap();
        let mut formatter = Formatter::new(&terminal).unwrap();
        assert_eq!(formatter.format_len().unwrap(), 0);
        assert_eq!(formatter.format_buf(&mut []).unwrap(), 0);
        assert!(formatter.format_alloc(None).unwrap().is_empty());
    }

    #[test]
    fn writer_output_matches_buffer_output_in_every_format() {
        let mut terminal = Terminal::new(20, 3).unwrap();
        terminal.vt_write(b"plain \x1b[1;31mbold red\x1b[0m\r\nsecond line");
        for format in [Format::Plain, Format::Vt, Format::Html] {
            let mut formatter = Formatter::new(&terminal).unwrap();
            formatter.set_format(format).unwrap();
            let expected = format_buf(&mut formatter);
            assert!(!expected.is_empty(), "{format:?}");

            let mut actual = Vec::new();
            format_write(&mut formatter, &mut actual).unwrap();
            assert_eq!(actual, expected, "{format:?}");
        }
    }

    /// A too-small buffer reports the size the output needs, so the caller
    /// can retry with exactly that much, as the docs promise.
    #[test]
    fn format_buf_reports_the_required_size() {
        let mut terminal = Terminal::new(10, 3).unwrap();
        terminal.vt_write(b"one\r\ntwo");
        let mut formatter = Formatter::new(&terminal).unwrap();

        // "one\ntwo" is 7 bytes.
        let mut buf = [0; 3];
        let err = formatter.format_buf(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutOfSpace { required: 7 }), "{err:?}");

        let mut buf = [0; 7];
        let len = formatter.format_buf(&mut buf).unwrap();
        assert_eq!(&buf[..len], b"one\ntwo");
    }

    /// Mirrors upstream's `PageList VT without trim keeps trailing blank
    /// rows` test in `src/terminal/formatter.zig`.
    #[test]
    fn only_untrimmed_vt_keeps_trailing_blank_rows() {
        let mut terminal = Terminal::new(10, 6).unwrap();
        terminal.vt_write(b"one\r\ntwo");
        let format = |format, trim| {
            let mut formatter = Formatter::new(&terminal).unwrap();
            formatter
                .set_format(format)
                .unwrap()
                .set_trim(trim)
                .unwrap()
                .set_extras(Extras::NONE)
                .unwrap();
            format_buf(&mut formatter)
        };

        // Six rows, two with text, so five newlines.
        assert_eq!(format(Format::Vt, false), b"one\r\ntwo\r\n\r\n\r\n\r\n");
        // Trimmed output and other formats still drop them.
        assert_eq!(format(Format::Vt, true), b"one\r\ntwo");
        assert_eq!(format(Format::Plain, false), b"one\ntwo");
    }

    /// Selection formatting keeps trailing blank rows only up to the last
    /// selected row, and only for untrimmed VT output.
    #[test]
    fn only_untrimmed_vt_keeps_trailing_blank_rows_in_selection() {
        use crate::terminal::{Point, PointCoordinate};

        let mut terminal = Terminal::new(10, 6).unwrap();
        terminal.vt_write(b"one\r\ntwo");
        let at = |x, y| {
            terminal
                .grid_ref(Point::Active(PointCoordinate { x, y }))
                .unwrap()
        };
        // Rows 0 to 3: two with text, then two blank ones.
        let selection = Selection::new(at(0, 0), at(9, 3), false);
        let format = |format, trim| {
            let mut formatter = Formatter::new(&terminal).unwrap();
            formatter
                .set_format(format)
                .unwrap()
                .set_trim(trim)
                .unwrap();
            // SAFETY: Both endpoints come from the formatter's active screen.
            unsafe { formatter.set_selection(Some(&selection)) }.unwrap();
            formatter.format_alloc(None).unwrap().to_vec()
        };
        // The default extras emit the whole 256-color palette as
        // OSC 4 sequences (each ending in ST) before it, and the SGR reset
        // after it. Strip them one by one, unwrapping at each step, so that
        // anything else (stray output, a missing entry) fails the test
        // instead of being silently cut off along with them.
        let vt_content = |trim| {
            let output = format(Format::Vt, trim);
            let mut rest = output.as_slice();
            for _ in 0..256 {
                rest = rest.strip_prefix(b"\x1b]4;").unwrap();
                let st = rest.windows(2).position(|w| w == b"\x1b\\").unwrap();
                rest = &rest[st + 2..];
            }
            rest.strip_suffix(b"\x1b[0m").unwrap().to_vec()
        };

        // Four rows, so three newlines: one between the text rows and one
        // for each blank row. The blank rows below the selection don't count.
        assert_eq!(vt_content(false), b"one\r\ntwo\r\n\r\n");
        assert_eq!(vt_content(true), b"one\r\ntwo");
        assert_eq!(format(Format::Plain, false), b"one\ntwo");
    }

    #[test]
    fn partial_writes_are_retried_until_complete() {
        let mut terminal = Terminal::new(20, 3).unwrap();
        terminal.vt_write(b"hello\r\nworld");
        let mut formatter = Formatter::new(&terminal).unwrap();
        let expected = format_buf(&mut formatter);

        let mut writer = ByteAtATime {
            bytes: Vec::new(),
            writes: 0,
        };
        format_write(&mut formatter, &mut writer).unwrap();
        assert_eq!(writer.bytes, expected);
        assert_eq!(writer.writes, expected.len());
    }

    #[test]
    fn writer_failure_is_an_io_error() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"hello");
        let mut formatter = Formatter::new(&terminal).unwrap();
        assert!(matches!(
            format_write(&mut formatter, &mut Reject),
            Err(Error::IoError)
        ));
        // The formatter stays usable afterwards.
        let mut output = Vec::new();
        format_write(&mut formatter, &mut output).unwrap();
        assert_eq!(output, b"hello");
    }

    /// A writer that accepts `chunks` writes, then rejects everything.
    struct FailAfter {
        chunks: usize,
        bytes: Vec<u8>,
    }

    impl std::io::Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.chunks == 0 {
                return Err(std::io::Error::other("rejected"));
            }
            self.chunks -= 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn large_output_streams_in_several_writes() {
        // Far more output than libghostty buffers before handing it over.
        let mut terminal = Terminal::new(200, 100).unwrap();
        for i in 0..100 {
            terminal.vt_write(format!("{i:0>199}\r\n").as_bytes());
        }
        let mut formatter = Formatter::new(&terminal).unwrap();
        let expected = format_buf(&mut formatter);
        assert!(expected.len() > 16 * 1024);

        let mut writer = FailAfter {
            chunks: usize::MAX,
            bytes: Vec::new(),
        };
        format_write(&mut formatter, &mut writer).unwrap();
        assert_eq!(writer.bytes, expected);
        assert!(usize::MAX - writer.chunks > 1, "output came in one write");

        // A writer that fails after the first chunk fails the whole call,
        // having received only part of the output.
        let mut writer = FailAfter {
            chunks: 1,
            bytes: Vec::new(),
        };
        assert!(matches!(
            format_write(&mut formatter, &mut writer),
            Err(Error::IoError)
        ));
        assert!(!writer.bytes.is_empty() && writer.bytes.len() < expected.len());
    }
}
