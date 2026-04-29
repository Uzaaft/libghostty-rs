//! Format terminal content as plain text, VT sequences, or HTML.
//!
//! A formatter captures a reference to a terminal and formatting options.
//! It can be used repeatedly to produce output that reflects the current
//! terminal state at the time of each format call.
use std::{marker::PhantomData, ptr::NonNull};

use crate::{
    alloc::{Allocator, Bytes, Object},
    error::{Error, Result, from_result},
    ffi,
    screen::Selection,
    terminal::Terminal,
};

/// Formatter that formats terminal content.
#[derive(Debug)]
pub struct Formatter<'t, 'alloc: 'cb, 'cb: 't> {
    inner: Object<'alloc, ffi::FormatterImpl>,
    _terminal: PhantomData<&'t Terminal<'alloc, 'cb>>,
}

/// Options for creating a terminal formatter.
#[derive(Debug)]
pub struct FormatterOptions<'t> {
    /// Output format to emit.
    pub format: Format,
    /// Whether to trim trailing whitespace on non-blank lines.
    pub trim: bool,
    /// Whether to unwrap soft-wrapped lines.
    pub unwrap: bool,
    /// Optional selection to restrict output to a range.
    ///
    /// If `None`, the entire screen is formatted.
    pub selection: Option<Selection<'t>>,
}

impl<'t, 'alloc: 'cb, 'cb: 't> Formatter<'t, 'alloc, 'cb> {
    /// Create a formatter for a terminal's active screen.
    pub fn new(terminal: &'t Terminal<'alloc, 'cb>, opts: FormatterOptions<'t>) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), terminal, opts) }
    }

    /// Create a formatter for a terminal's active screen.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        terminal: &'t Terminal<'alloc, 'cb>,
        opts: FormatterOptions,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), terminal, opts) }
    }

    unsafe fn new_inner(
        alloc: *const ffi::Allocator,
        terminal: &'t Terminal<'alloc, 'cb>,
        opts: FormatterOptions,
    ) -> Result<Self> {
        let mut raw: ffi::Formatter = std::ptr::null_mut();
        let selection = opts.selection.map(Into::into);

        let opts = ffi::FormatterTerminalOptions {
            emit: opts.format.into(),
            trim: opts.trim,
            extra: ffi::FormatterTerminalExtra::default(),
            unwrap: opts.unwrap,
            selection: match selection {
                Some(s) => &raw const s,
                None => std::ptr::null(),
            },
            ..ffi::sized!(ffi::FormatterTerminalOptions)
        };

        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(
                alloc,
                &raw mut raw,
                terminal.inner.as_raw(),
                opts.into(),
            )
        };
        from_result(result)?;

        Ok(Self {
            inner: Object::new(raw)?,
            _terminal: PhantomData,
        })
    }

    /// Run the formatter and return an allocated buffer with the output.
    ///
    /// Each call formats the current terminal state. The buffer is allocated
    /// using the provided allocator (or the default allocator if `None`).
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

        let ptr = NonNull::new(bytes).ok_or(Error::OutOfMemory)?;
        Ok(unsafe { Bytes::from_raw_parts(ptr, len, alloc) })
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
        from_result(result)?;
        Ok(len)
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
        // This should always fail with OutOfSpace.
        match from_result(result) {
            Err(Error::OutOfSpace { .. }) => Ok(len),
            Err(e) => Err(e),
            Ok(()) => Err(Error::InvalidValue),
        }
    }
}

impl Drop for Formatter<'_, '_, '_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.inner.as_raw()) }
    }
}

/// Output format.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
pub enum Format {
    /// Plain text (no escape sequences).
    Plain = ffi::FormatterFormat::PLAIN,
    /// VT sequences preserving colors, styles, URLs, etc.
    Vt = ffi::FormatterFormat::VT,
    /// HTML with inline styles.
    Html = ffi::FormatterFormat::HTML,
}
