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
    terminal::Terminal,
};

/// Formatter that formats terminal content.
#[derive(Debug)]
pub struct Formatter<'t, 'alloc, 'ud, UserData> {
    inner: Object<'alloc, ffi::GhosttyFormatter>,
    _terminal: PhantomData<&'t Terminal<'alloc, 'ud, UserData>>,
}

/// Options for creating a terminal formatter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatterOptions {
    /// Output format to emit.
    pub format: Format,
    /// Whether to trim trailing whitespace on non-blank lines.
    pub trim: bool,
    /// Whether to unwrap soft-wrapped lines.
    pub unwrap: bool,
}

impl<'t, 'alloc: 'ud, 'ud, UserData: 'ud> Formatter<'t, 'alloc, 'ud, UserData> {
    /// Create a formatter for a terminal's active screen.
    pub fn new(
        terminal: &'t Terminal<'alloc, 'ud, UserData>,
        opts: FormatterOptions,
    ) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), terminal, opts) }
    }

    /// Create a formatter for a terminal's active screen.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(
        alloc: &'alloc Allocator<'ctx, Ctx>,
        terminal: &'t Terminal<'alloc, 'ud, UserData>,
        opts: FormatterOptions,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), terminal, opts) }
    }

    unsafe fn new_inner(
        alloc: *const ffi::GhosttyAllocator,
        terminal: &'t Terminal<'alloc, 'ud, UserData>,
        opts: FormatterOptions,
    ) -> Result<Self> {
        let mut raw: ffi::GhosttyFormatter_ptr = std::ptr::null_mut();
        let result = unsafe {
            ffi::ghostty_formatter_terminal_new(
                alloc,
                &mut raw,
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

    // Run the formatter and return an allocated buffer with the output.
    //
    // Each call formats the current terminal state. The buffer is allocated
    // using the provided allocator (or the default allocator if `None`).
    pub fn format_alloc<'a, 'ctx: 'a, Ctx>(
        &mut self,
        alloc: Option<&'a Allocator<'ctx, Ctx>>,
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
            Ok(_) => Err(Error::InvalidValue),
        }
    }
}

impl<UserData> Drop for Formatter<'_, '_, '_, UserData> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.inner.as_raw()) }
    }
}

impl From<FormatterOptions> for ffi::GhosttyFormatterTerminalOptions {
    fn from(value: FormatterOptions) -> Self {
        Self {
            size: std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>(),
            emit: value.format.into(),
            trim: value.trim,
            extra: ffi::GhosttyFormatterTerminalExtra::default(),
            unwrap: value.unwrap,
        }
    }
}

/// Output format.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, int_enum::IntEnum)]
pub enum Format {
    /// Plain text (no escape sequences).
    Plain = ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN,
    /// VT sequences preserving colors, styles, URLs, etc.
    Vt = ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_VT,
    /// HTML with inline styles.
    Html = ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_HTML,
}
